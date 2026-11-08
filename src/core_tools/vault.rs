//! The Vault's tools, and what they ask the node (`/v1/vault…`).

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use super::{Asked, Feature, Request, Tool, its_said, text};

const GUIDE: &str = "The Vault keeps the user's keys, sign-ins and accounts as items, each a page's (theirs, a group's, everyone's): fields (secret ones never read back), the sites they're sent to, and the apps switched on for it. Never ask the user to paste a secret (an API key, a password, a token) into the chat, and never paste one yourself. To keep one, call vault_request and give them its link: they pick one of their items or type a new one into a sheet on their own device, and you never see it. To use one yourself, ask leaving out app, then call vault_fetch with {{<field>}} where it goes: a machine that opens the Vault fills it in, only for the item's sites, and you get the answer. If someone pastes a secret anyway, keep it with vault_set and don't repeat it. vault_list shows what's there.";

pub fn feature() -> Feature {
    Feature {
        name: "vault",
        title: "Vault",
        description: "Keys, sign-ins and accounts, used without being seen".into(),
        guide: GUIDE.into(),
        tools: tools(),
        #[cfg(feature = "node")]
        steps: steps::run,
    }
}

/// The Vault is the node's, not an app's: agents list its items (never a secret field's value), set and remove
/// fields, switch apps on for items, and ask the user for one.
fn tools() -> Vec<Tool> {
    let scope = json!({ "type": "string", "description": "user (yours, the default), global (everyone's), or a group's name or id" });
    let item = json!({ "type": "string", "description": "the item's title or id" });
    vec![
        Tool::steps(json!({ "name": "list", "reads": true, "description": "The Vault's items you may change: title, type, fields (a secret one never with its value), the sites they're sent to (hosts), the apps switched on for each (apps) and those that want it (wanted_by), and app when an app's sign-in keeps it.", "input": { "type": "object", "properties": { "scope": scope } } })),
        Tool::steps(json!({ "name": "set", "description": "Keep a key, token or password someone pasted (or any field) in an item, making the item if no item has that title. An app puts {{<item id>:<field>}} in a request and a machine that opens the Vault fills it in, a secret field only for the item's hosts; nobody reads a secret back, not you either. When the user has the value, use vault_request instead: never ask them to paste it into the chat.", "input": { "type": "object", "required": ["item", "field", "value"], "properties": { "item": item, "field": { "type": "string", "description": "the field's name, like token or password" }, "value": { "type": "string", "description": "the field's value" }, "secret": { "type": "boolean", "description": "never shown again; true for keys, passwords and tokens" }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "app": { "type": "string", "description": "an app to switch on for it" }, "scope": scope } } })),
        Tool::steps(json!({ "name": "use", "description": "Switch an app on (or off) for an item, so its calls may use the item's fields. Only the user switches agents on (for vault_fetch): ask them with vault_request.", "input": { "type": "object", "required": ["item", "app"], "properties": { "item": item, "app": { "type": "string", "description": "the app's name, like gmail" }, "on": { "type": "boolean", "description": "false switches it off" }, "scope": scope } } })),
        Tool::asks(json!({ "name": "grant", "description": format!("Let a machine of {} open a Vault's secrets and fill them into requests (or, with on false, no longer). Only the Vault's admins say which: the user for their own, a group's admins, the network's for everyone's. Nobody else reads a secret back, not even on their own device.", crate::config::YOURS), "input": { "type": "object", "required": ["device"], "properties": { "device": { "type": "string", "description": "the machine's name" }, "on": { "type": "boolean", "description": "false: it no longer opens it" }, "scope": scope } } }), grant, its_said),
        Tool::steps(json!({ "name": "remove", "confirm": true, "words": ["item", "field"], "description": "Delete an item, or with field, one of its fields.", "input": { "type": "object", "required": ["item"], "properties": { "item": item, "field": { "type": "string", "description": "only this field, by name; the whole item when not given" }, "scope": scope } } })),
        Tool::asks(json!({ "name": "request", "words": ["app"], "description": "Ask the user for a key, token, password or sign-in an app or agents may use, instead of handling its values yourself: need names what the app declares (its vault), or give title, fields and hosts. Returns a link to give them; it opens a sheet where they pick one of their items or make one (and their Settings lists it). You never see a secret. Tell the user; check vault_status when they say they've done it.", "input": { "type": "object", "properties": { "app": { "type": "string", "description": "the app that will use it; leave it out for one you use yourself with vault_fetch" }, "need": { "type": "string", "description": "what the app declares it needs, by name, like client for gmail; leave it out and give title, fields and hosts for anything else" }, "title": { "type": "string", "description": "the item's title, for one no app declares" }, "fields": { "type": "object", "description": "field name → { secret: true|false }", "additionalProperties": { "type": "object", "properties": { "secret": { "type": "boolean" } } } }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "why": { "type": "string", "description": "one line the user sees: what it's for and where to find it" }, "scope": scope } } }), request, request_said),
        Tool::asks(json!({ "name": "fetch", "description": "Make an HTTP request with your user's Vault items filled in, without seeing them: write {{<field>}} (or {{<item id>:<field>}}) where a value goes, in url, headers or body, and a machine that opens the Vault fills it in from the items they let agents use, a secret field only for its item's sites. You get { status, headers, body }; a secret the site echoes back is blotted out. To get an item, ask with vault_request, leaving out app.", "input": { "type": "object", "required": ["url"], "properties": { "url": { "type": "string", "description": "the URL, with {{<field>}} where a value goes" }, "method": { "type": "string", "description": "GET by default, POST with a body" }, "headers": { "type": "object", "additionalProperties": { "type": "string" }, "description": "like { \"authorization\": \"Bearer {{token}}\" }" }, "body": { "type": "string", "description": "the request's body, with {{<field>}} where a value goes" } } } }), fetch, super::as_given),
        Tool::asks(json!({ "name": "status", "reads": true, "description": "How a request for an item went: waiting, set, declined or expired, and who answered. Never a value.", "input": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "description": "the request's id, from vault_request" } } } }), status, status_said),
    ]
}

/// The scope its input names: user, the default, global, or a group's name or id.
fn scope(input: &Map<String, Value>) -> &str {
    input.get("scope").and_then(Value::as_str).unwrap_or("user")
}

fn grant(input: &Map<String, Value>, _: &Asked) -> Result<Request> {
    Ok(Request::post("/v1/vault/grant", json!({ "scope": scope(input), "device": text(input, "device")?, "on": input.get("on") != Some(&json!(false)) })))
}

fn fetch(input: &Map<String, Value>, asked: &Asked) -> Result<Request> {
    Ok(Request::post("/v1/vault/fetch", json!({ "url": text(input, "url")?, "method": input.get("method"), "headers": input.get("headers"), "body": input.get("body"), "agent": asked.agent })))
}

/// Asks for an item for an app, or with none, for agents.
fn request(input: &Map<String, Value>, asked: &Asked) -> Result<Request> {
    let app = input.get("app").and_then(Value::as_str).unwrap_or(crate::database::scopes::AGENTS);
    Ok(Request::post("/v1/vault/request", json!({ "scope": scope(input), "app": app, "need": input.get("need"), "title": input.get("title"), "fields": input.get("fields"), "hosts": input.get("hosts"), "why": input.get("why"), "agent": asked.agent })))
}

fn request_said(_: &Map<String, Value>, r: Value) -> Result<Value> {
    Ok(json!(format!("Asked. Give the user this link: {}\nIt's also waiting in their Settings, under Requests. Tell them; check vault_status with id {} when they say they've done it.", r["link"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""))))
}

fn status(input: &Map<String, Value>, _: &Asked) -> Result<Request> {
    Ok(asking_for(text(input, "id")?))
}

fn status_said(input: &Map<String, Value>, r: Value) -> Result<Value> {
    let r = the_request(text(input, "id")?, r)?;
    Ok(json!({ "state": r["state"], "app": r["app"], "title": r["title"], "answered": r["answered"] }))
}

/// What asks for the request `id`, whose answer `the_request` reads.
pub fn asking_for(id: &str) -> Request {
    Request::get(format!("/v1/vault/requests?id={id}"))
}

pub fn the_request(id: &str, answer: Value) -> Result<Value> {
    answer.as_array().and_then(|a| a.first()).cloned().with_context(|| format!("no request {id}"))
}

#[cfg(feature = "node")]
mod steps {
    use anyhow::{Context, Result, bail};
    use serde_json::{Map, Value, json};

    use super::{Asked, scope, text};
    use crate::api::Client;
    use crate::config::CLI;

    pub fn run(tool: &str, input: &Map<String, Value>, asked: &Asked, node: &Client) -> Result<Value> {
        let (scope, agent) = (scope(input), asked.agent);
        let text = |k: &str| text(input, k);
        match tool {
            "list" => Ok(json!(node.get(&format!("/v1/vault?scope={scope}"))?.as_array().into_iter().flatten().map(item_said).collect::<Vec<_>>())),
            "set" => {
                let hosts = input.get("hosts").and_then(Value::as_array).map(|h| h.iter().filter_map(Value::as_str).map(str::to_string).collect());
                Ok(json!(set(node, scope, text("item")?, text("field")?, text("value")?, input.get("secret").and_then(Value::as_bool), hosts, input.get("app").and_then(Value::as_str), agent)?))
            }
            "use" => Ok(json!(switch(node, scope, text("item")?, text("app")?, input.get("on") != Some(&json!(false)), agent)?)),
            "remove" => Ok(json!(remove(node, scope, text("item")?, input.get("field").and_then(Value::as_str))?)),
            other => bail!("no tool vault_{other}"),
        }
    }

    /// The apps switched on for an item, as a user says them: Agents for agents.
    fn used_by(apps: &Value) -> Vec<&str> {
        apps.as_array().into_iter().flatten().filter_map(Value::as_str).map(|a| if a == crate::database::scopes::AGENTS { "Agents" } else { a }).collect()
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
    fn find_item(node: &Client, scope: &str, name: &str) -> Result<Option<Value>> {
        let items = node.get(&format!("/v1/vault?scope={scope}"))?;
        let found: Vec<Value> = items.as_array().into_iter().flatten().filter(|i| i["id"] == name || i["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(name))).cloned().collect();
        anyhow::ensure!(found.len() < 2, "several items are called {name}: give its id");
        Ok(found.into_iter().next())
    }

    /// Sets `field` of `item` in `scope` (a new item by that title if there's none), secret if asked or if it was.
    #[allow(clippy::too_many_arguments)]
    pub fn set(node: &Client, scope: &str, item: &str, field: &str, value: &str, secret: Option<bool>, hosts: Option<Vec<String>>, app: Option<&str>, agent: Option<&str>) -> Result<String> {
        let found = find_item(node, scope, item)?;
        let was = found.as_ref().and_then(|it| it["fields"].as_array()?.iter().find(|f| f["name"] == field).map(|f| f["secret"] == true));
        let mut data = json!({ "scope": scope, "agent": agent, "fields": { field: { "value": value, "secret": secret.or(was).unwrap_or(false) } } });
        if let Some(hosts) = hosts {
            data["hosts"] = json!(hosts);
        }
        let made = match &found {
            Some(it) => {
                data["id"] = it["id"].clone();
                if let Some(app) = app {
                    let mut apps: Vec<Value> = it["apps"].as_array().cloned().unwrap_or_default();
                    if !apps.contains(&json!(app)) {
                        apps.push(json!(app));
                    }
                    data["apps"] = json!(apps);
                }
                false
            }
            None => {
                data["title"] = json!(item);
                data["apps"] = json!(app.into_iter().collect::<Vec<_>>());
                true
            }
        };
        let id = node.post("/v1/vault", data)?["id"].as_str().unwrap_or("").to_string();
        let done = match made {
            true => format!("Made {item} ({id}) in {}, with {field}.", vault_name(scope)),
            false => format!("Set {item}'s {field}."),
        };
        // Whether an agent may use it now, since only the user lets agents use an item.
        let agents = found.as_ref().is_some_and(|it| it["apps"].as_array().is_some_and(|a| a.iter().any(|a| a == crate::database::scopes::AGENTS))) || app == Some(crate::database::scopes::AGENTS);
        Ok(match (agents, app) {
            (true, _) => format!("{done} Agents use it with vault_fetch, as {{{{{id}:{field}}}}}."),
            (false, None) => format!("{done} Agents may not use it yet: {} asks the user to let you, and they pick it.", crate::node::vault::ASK_FOR_AGENTS),
            (false, Some(_)) => done,
        })
    }

    pub fn switch(node: &Client, scope: &str, item: &str, app: &str, on: bool, agent: Option<&str>) -> Result<String> {
        let it = find_item(node, scope, item)?.with_context(|| format!("no item {item} in {scope}"))?;
        let mut apps: Vec<Value> = it["apps"].as_array().cloned().unwrap_or_default().into_iter().filter(|a| a != app).collect();
        if on {
            apps.push(json!(app));
        }
        node.post("/v1/vault", json!({ "scope": scope, "id": it["id"], "apps": apps, "agent": agent }))?;
        Ok(format!("{} is switched {} for {}.", used_by(&json!([app]))[0], if on { "on" } else { "off" }, it["title"].as_str().unwrap_or(item)))
    }

    /// A scope's Vault as a user calls it: your Vault, Everyone's Vault, <name>'s Vault.
    fn vault_name(scope: &str) -> String {
        match scope {
            "user" => "your Vault".to_string(),
            "global" => "Everyone's Vault".to_string(),
            group => format!("{group}'s Vault"),
        }
    }

    /// Deletes `item` in `scope`, or with `field`, one of its fields. When it's not there but is in another Vault you
    /// may change, says which and gives the command that removes it.
    pub fn remove(node: &Client, scope: &str, item: &str, field: Option<&str>) -> Result<String> {
        let it = match find_item(node, scope, item)? {
            Some(it) => it,
            None => {
                let all = node.get("/v1/vault")?.as_array().cloned().unwrap_or_default();
                let where_it: Vec<String> = all
                    .iter()
                    .filter(|i| i["id"] == item || i["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(item)))
                    .map(|it| {
                        let s = it["scope"].as_str().unwrap_or("");
                        let (flag, vault) = match s {
                            s if s.starts_with("user:") => ("user".to_string(), vault_name("user")),
                            s if s.starts_with("group:") => (s.to_string(), vault_name(it["scope_name"].as_str().unwrap_or(s))),
                            _ => (s.to_string(), vault_name(s)),
                        };
                        format!("in {vault}: {CLI} vault remove {}{} --scope {flag}", it["id"].as_str().unwrap_or(item), field.map(|f| format!(" {f}")).unwrap_or_default())
                    })
                    .collect();
                let there = match where_it.is_empty() {
                    true => String::new(),
                    false => format!("; it's {}", where_it.join(", and ")),
                };
                bail!("no item {item} in {}{there}", vault_name(scope));
            }
        };
        let title = it["title"].as_str().unwrap_or(item);
        match field {
            Some(field) => {
                node.post("/v1/vault", json!({ "scope": scope, "id": it["id"], "fields": { field: null } }))?;
                Ok(format!("Removed {title}'s {field}."))
            }
            None => {
                node.post("/v1/vault", json!({ "scope": scope, "id": it["id"], "deleted": true }))?;
                Ok(format!("Deleted {title}."))
            }
        }
    }
}
