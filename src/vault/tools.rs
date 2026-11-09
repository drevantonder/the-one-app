//! The Vault's tools, each its own route (`core_tools::Runs::Route`): what it takes and answers, beside what serves it
//! on a node (`serve`). `grant` changes who can do what, so the caller's client answers it (`settings::answered`), and
//! its route is only this machine's own programs', whose node answers it as its own device.

use serde_json::{Value, json};

use crate::core_tools::{Feature, Tool, serves};

const GUIDE: &str = "The Vault keeps the user's keys, sign-ins and accounts as items, each a page's (theirs, a group's, everyone's): fields (secret ones never read back), the sites they're sent to, and the apps switched on for it. Never ask the user to paste a secret (an API key, a password, a token) into the chat, and never paste one yourself. To keep one, call vault_request and give them its link: they pick one of their items or type a new one into a sheet on their own device, and you never see it. To use one yourself, ask leaving out app, then call vault_fetch with {{<field>}} where it goes: a machine that opens the Vault fills it in, only for the item's sites, and you get the answer. If someone pastes a secret anyway, keep it with vault_set and don't repeat it. vault_list shows what's there.";

pub fn feature() -> Feature {
    Feature {
        name: "vault",
        title: "Vault",
        description: "Keys, sign-ins and accounts, used without being seen".into(),
        guide: GUIDE.into(),
        tools: tools(),
        #[cfg(feature = "node")]
        steps: crate::core_tools::no_steps,
    }
}

/// An answer that's only words for whoever asked.
fn said(about: &str) -> Value {
    json!({ "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": about } } })
}

/// The Vault is the node's, not an app's: agents list its items (never a secret field's value), set and remove
/// fields, switch apps on for items, and ask the user for one.
fn tools() -> Vec<Tool> {
    let scope = json!({ "type": "string", "description": "user (yours, the default), global (everyone's), or a group's name or id" });
    let item = json!({ "type": "string", "description": "the item's title or id" });
    let strings = json!({ "type": "array", "items": { "type": "string" } });
    vec![
        Tool::route(
            json!({
                "name": "list", "title": "List the Vault's items", "reads": true,
                "description": "The Vault's items you may change: title, type, fields (a secret one never with its value), the sites they're sent to (hosts), the apps switched on for each (apps) and those that want it (wanted_by), and app when an app's sign-in keeps it.",
                "input": { "type": "object", "properties": { "scope": scope } },
                "output": { "type": "array", "items": { "type": "object", "required": ["id", "title", "page", "fields", "hosts", "apps", "wanted_by", "by"], "properties": {
                    "id": { "type": "string" }, "title": { "type": "string" }, "page": { "type": "string", "description": "whose Vault it's in" },
                    "fields": { "type": "array", "items": { "type": "object" }, "description": "each { name, secret }, and a value only when it isn't secret" },
                    "hosts": strings, "apps": strings, "wanted_by": strings,
                    "changed": { "type": ["string", "null"], "description": "when it last changed, ISO 8601" }, "by": { "type": "string", "description": "who changed it last" },
                    "type": { "type": "string" }, "domain": { "type": "string" }, "app": { "type": "string", "description": "the app whose sign-in keeps it" }, "account": { "type": "boolean" },
                } } },
            }),
            serves!(serve::list),
        ),
        Tool::route(
            json!({
                "name": "set", "title": "Keep a secret in the Vault",
                "description": "Keep a key, token or password someone pasted (or any field) in an item, making the item if no item has that title. An app puts {{<item id>:<field>}} in a request and a machine that opens the Vault fills it in, a secret field only for the item's hosts; nobody reads a secret back, not you either. When the user has the value, use vault_request instead: never ask them to paste it into the chat.",
                "input": { "type": "object", "required": ["item", "field", "value"], "properties": { "item": item, "field": { "type": "string", "description": "the field's name, like token or password" }, "value": { "type": "string", "description": "the field's value" }, "secret": { "type": "boolean", "description": "never shown again; true for keys, passwords and tokens" }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "app": { "type": "string", "description": "an app to switch on for it" }, "scope": scope } },
                "output": said("what was set, and how agents use it"),
            }),
            serves!(serve::set),
        ),
        Tool::route(
            json!({
                "name": "use", "title": "Switch an app on for an item",
                "description": "Switch an app on (or off) for an item, so its calls may use the item's fields. Only the user switches agents on (for vault_fetch): ask them with vault_request.",
                "input": { "type": "object", "required": ["item", "app"], "properties": { "item": item, "app": { "type": "string", "description": "the app's name, like gmail" }, "on": { "type": "boolean", "description": "false switches it off" }, "scope": scope } },
                "output": said("what's switched on or off"),
            }),
            serves!(serve::switch),
        ),
        Tool::route(
            json!({
                "name": "grant", "title": "Let a machine open a Vault",
                "description": format!("Let a machine of {} open a Vault's secrets and fill them into requests (or, with on false, no longer). Only the Vault's admins say which: the user for their own, a group's admins, the network's for everyone's. Nobody else reads a secret back, not even on their own device.", crate::config::YOURS),
                "input": { "type": "object", "required": ["device"], "properties": { "device": { "type": "string", "description": "the machine's name" }, "on": { "type": "boolean", "description": "false: it no longer opens it" }, "scope": scope } },
                "output": said("which machine opens the Vault now"),
            }),
            serves!(serve::grant),
        ),
        Tool::route(
            json!({
                "name": "remove", "title": "Delete an item or a field", "confirm": true, "words": ["item", "field"],
                "description": "Delete an item, or with field, one of its fields.",
                "input": { "type": "object", "required": ["item"], "properties": { "item": item, "field": { "type": "string", "description": "only this field, by name; the whole item when not given" }, "scope": scope } },
                "output": said("what was deleted"),
            }),
            serves!(serve::remove),
        ),
        Tool::route(
            json!({
                "name": "request", "title": "Ask the user for a secret", "words": ["app"],
                "description": "Ask the user for a key, token, password or sign-in an app or agents may use, instead of handling its values yourself: need names what the app declares (its vault), or give title, fields and hosts. Returns a link to give them; it opens a sheet where they pick one of their items or make one (and their Settings lists it). You never see a secret. Tell the user; check vault_status when they say they've done it.",
                "input": { "type": "object", "properties": { "app": { "type": "string", "description": "the app that will use it; leave it out for one you use yourself with vault_fetch" }, "need": { "type": "string", "description": "what the app declares it needs, by name, like client for gmail; leave it out and give title, fields and hosts for anything else" }, "title": { "type": "string", "description": "the item's title, for one no app declares" }, "fields": { "type": "object", "description": "field name → { secret: true|false }", "additionalProperties": { "type": "object", "properties": { "secret": { "type": "boolean" } } } }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "why": { "type": "string", "description": "one line the user sees: what it's for and where to find it" }, "scope": scope } },
                "output": { "type": "object", "required": ["id", "scope", "link", "said"], "properties": {
                    "id": { "type": "string", "description": "the request's, for vault_status" }, "scope": { "type": "string" },
                    "link": { "type": "string", "description": "where the user answers it" }, "said": { "type": "string", "description": "what to tell the user" },
                } },
            }),
            serves!(serve::request),
        ),
        Tool::route(
            json!({
                "name": "fetch", "title": "Make a request with Vault items filled in",
                "description": "Make an HTTP request with your user's Vault items filled in, without seeing them: write {{<field>}} (or {{<item id>:<field>}}) where a value goes, in url, headers or body, and a machine that opens the Vault fills it in from the items they let agents use, a secret field only for its item's sites. You get { status, headers, body }; a secret the site echoes back is blotted out. To get an item, ask with vault_request, leaving out app.",
                "input": { "type": "object", "required": ["url"], "properties": { "url": { "type": "string", "description": "the URL, with {{<field>}} where a value goes" }, "method": { "type": "string", "description": "GET by default, POST with a body" }, "headers": { "type": "object", "additionalProperties": { "type": "string" }, "description": "like { \"authorization\": \"Bearer {{token}}\" }" }, "body": { "type": "string", "description": "the request's body, with {{<field>}} where a value goes" } } },
                "output": { "type": "object", "required": ["status", "headers"], "properties": {
                    "status": { "type": "integer" }, "headers": { "type": "object", "additionalProperties": { "type": "string" } },
                    "body": { "type": "string", "description": "as text, when it is" }, "body_base64": { "type": "string", "description": "when it isn't text" },
                    "cut": { "type": "string", "description": "how much was left out, when it was long" },
                } },
            }),
            serves!(serve::fetch),
        ),
        Tool::route(
            json!({
                "name": "status", "title": "How a request for an item went", "reads": true,
                "description": "How a request for an item went: waiting, set, declined or expired, and who answered. Never a value.",
                "input": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "description": "the request's id, from vault_request" } } },
                "output": { "type": "object", "required": ["state"], "properties": {
                    "state": { "type": "string", "description": "waiting, expired, or how it was answered" }, "app": { "type": "string" }, "title": { "type": "string" },
                    "answered": { "type": "object", "properties": { "by": { "type": "string" }, "time": { "type": "integer" } }, "description": "who answered it, and when, in ms since 1970" },
                } },
            }),
            serves!(serve::status),
        ),
    ]
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::{Context, Result, bail};
    use serde_json::{Map, Value, json};

    use crate::config::CLI;
    use crate::core_tools::text;
    use crate::database::scopes::{AGENTS, Scope};
    use crate::node::{Caller, Node};

    /// The scope its input names: user, the default, global, or a group's name or id.
    fn scope(node: &Node, me: &Caller, input: &Map<String, Value>) -> Result<Scope> {
        node.scope_for(me, input.get("scope").and_then(Value::as_str).unwrap_or("user"))
    }

    pub async fn list(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let scope = scope(&node, &me, &input)?;
        Ok(json!(node.vault(&me, Some(&scope)).await?.iter().map(item_said).collect::<Vec<_>>()))
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
    async fn find_item(node: &Node, me: &Caller, scope: &Scope, name: &str) -> Result<Option<Value>> {
        let found: Vec<Value> = node.vault(me, Some(scope)).await?.into_iter().filter(|i| i["id"] == name || i["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(name))).collect();
        anyhow::ensure!(found.len() < 2, "several items are called {name}: give its id");
        Ok(found.into_iter().next())
    }

    /// Sets `field` of `item` (a new item by that title if there's none), secret if asked or if it was.
    pub async fn set(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let (scope, item, field) = (scope(&node, &me, &input)?, text(&input, "item")?, text(&input, "field")?);
        let app = input.get("app").and_then(Value::as_str);
        let found = find_item(&node, &me, &scope, item).await?;
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
        let id = node.set_item(&me, &scope, data).await?;
        let done = match found {
            None => format!("Made {item} ({id}) in {}, with {field}.", vault_name(&input)),
            Some(_) => format!("Set {item}'s {field}."),
        };
        // Whether an agent may use it now, since only the user lets agents use an item.
        let agents = found.as_ref().is_some_and(|it| it["apps"].as_array().is_some_and(|a| a.iter().any(|a| a == AGENTS))) || app == Some(AGENTS);
        let said = match (agents, app) {
            (true, _) => format!("{done} Agents use it with vault_fetch, as {{{{{id}:{field}}}}}."),
            (false, None) => format!("{done} Agents may not use it yet: {} asks the user to let you, and they pick it.", crate::node::vault::ASK_FOR_AGENTS),
            (false, Some(_)) => done,
        };
        Ok(json!({ "said": said }))
    }

    pub async fn switch(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let (scope, item, app, on) = (scope(&node, &me, &input)?, text(&input, "item")?, text(&input, "app")?, input.get("on") != Some(&json!(false)));
        let it = find_item(&node, &me, &scope, item).await?.with_context(|| format!("no item {item} in {}", vault_name(&input)))?;
        let mut apps: Vec<Value> = it["apps"].as_array().cloned().unwrap_or_default().into_iter().filter(|a| a != app).collect();
        if on {
            apps.push(json!(app));
        }
        node.set_item(&me, &scope, json!({ "id": it["id"], "apps": apps })).await?;
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

    /// Asked of this machine's node by its own programs, which answers it as its own device (`settings`), then seals
    /// what's in that Vault to the new key if it opens it.
    pub async fn grant(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let said = node.answer_as_mine("POST", "/v1/vault/grant", &serde_json::to_vec(&input)?).await.map_err(|e| e.0)?.0;
        let scope = scope(&node, &me, &input)?;
        if let Err(err) = node.reseal_vault(&scope).await {
            tracing::warn!("sealing {scope}'s Vault to its new key: {err:#}");
        }
        Ok(said)
    }

    /// Deletes `item`, or with `field`, one of its fields. When it's not there but is in another Vault you may change,
    /// says which and gives the command that removes it.
    pub async fn remove(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let (scope, item, field) = (scope(&node, &me, &input)?, text(&input, "item")?, input.get("field").and_then(Value::as_str));
        let Some(it) = find_item(&node, &me, &scope, item).await? else {
            let where_it: Vec<String> = node
                .vault(&me, None)
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
                node.set_item(&me, &scope, json!({ "id": it["id"], "fields": { field: null } })).await?;
                format!("Removed {title}'s {field}.")
            }
            None => {
                node.set_item(&me, &scope, json!({ "id": it["id"], "deleted": true })).await?;
                format!("Deleted {title}.")
            }
        };
        Ok(json!({ "said": said }))
    }

    /// Asks the user for an item for an app, or with none, for agents.
    pub async fn request(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let scope = scope(&node, &me, &input)?;
        let app = input.get("app").and_then(Value::as_str).unwrap_or(AGENTS);
        let ask = json!({ "title": input.get("title"), "fields": input.get("fields"), "hosts": input.get("hosts") });
        let why = input.get("why").and_then(Value::as_str).unwrap_or("");
        let mut asked = node.request_item(&me, &scope, app, input.get("need").and_then(Value::as_str), &ask, why).await?;
        asked["said"] = json!(format!(
            "Asked. Give the user this link: {}\nIt's also waiting in their Settings, under Requests. Tell them; check vault_status with id {} when they say they've done it.",
            asked["link"].as_str().unwrap_or(""),
            asked["id"].as_str().unwrap_or("")
        ));
        Ok(asked)
    }

    pub async fn fetch(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        node.vault_fetch(&me, &Value::Object(input)).await
    }

    pub async fn status(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let id = text(&input, "id")?;
        let asked = node.item_requests(&me, None, Some(id)).await?;
        let r = asked.first().with_context(|| format!("no request {id}"))?;
        Ok(json!({ "state": r["state"], "app": r["app"], "title": r["title"], "answered": r["answered"] }))
    }
}
