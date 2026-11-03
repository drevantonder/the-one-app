//! The Vault: items, each in one scope, with named fields (secret ones set, never read back), the hosts its secret
//! fields go to, and the apps switched on for it. Apps write `{{<item id>:field}}` (or `{{field}}`) where a field
//! goes, and the node fills it in on the way out: into a web request to one of its hosts, or into the input of a call
//! to a sidecar whose name is one of them.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::entry::is_id;

/// An item: what its entries add up to.
#[derive(Clone, Default)]
pub struct Item {
    pub title: String,
    pub kind: String,
    pub domain: Option<String>,
    pub fields: BTreeMap<String, Field>,
    pub hosts: Vec<String>,
    pub apps: BTreeSet<String>,
    /// The app whose call made it (a sign-in's account), if one did.
    pub app: Option<String>,
    /// Who changed it last, and when; and when it was made.
    pub user: String,
    pub time: u64,
    pub made: u64,
}

#[derive(Clone)]
pub struct Field {
    pub value: String,
    pub secret: bool,
}

impl std::fmt::Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fields: Vec<String> = self.fields.iter().map(|(n, v)| if v.secret { format!("{n}: ••••") } else { format!("{n}: {:?}", v.value) }).collect();
        write!(f, "Item {{ {:?}, fields: [{}], hosts: {:?}, apps: {:?} }}", self.title, fields.join(", "), self.hosts, self.apps)
    }
}

/// A field's name, as a placeholder names it: letters, digits, `_`, `-` and `.`.
pub fn valid_field(name: &str) -> bool {
    crate::members::valid_secret_name(name)
}

/// Applies an `item` entry's data, written by `user` at `time`, to `items`. `known` says whether an app name may be
/// switched on (it isn't erased). Fails, changing nothing, if it doesn't make sense.
pub fn apply(items: &mut BTreeMap<String, Item>, data: &Value, user: &str, time: u64, known: impl Fn(&str) -> bool) -> Result<String> {
    let id = data["id"].as_str().filter(|id| is_id(id)).context("an item's id is 16 hex characters")?.to_string();
    if data["deleted"] == true {
        ensure!(items.remove(&id).is_some(), "no such item");
        return Ok(id);
    }
    let mut item = match items.get(&id) {
        Some(item) => item.clone(),
        None => Item { made: time, app: data["app"].as_str().map(str::to_string), ..Default::default() },
    };
    if let Some(title) = data["title"].as_str() {
        item.title = title.trim().to_string();
    }
    ensure!(!item.title.is_empty(), "an item has a title");
    if let Some(kind) = data["type"].as_str() {
        item.kind = kind.trim().to_string();
    }
    if !data["domain"].is_null() {
        item.domain = data["domain"].as_str().filter(|d| crate::members::valid_host(d)).map(str::to_string);
    }
    if let Some(fields) = data["fields"].as_object() {
        let before = item.fields.clone();
        for (name, field) in fields {
            ensure!(valid_field(name), "a field's name is letters, digits, _, - and .");
            if field.is_null() {
                item.fields.remove(name);
                continue;
            }
            let value = match field["from"].as_str() {
                Some(from) => before.get(from).with_context(|| format!("no field {from} to take"))?.value.clone(),
                None => field["value"].as_str().context("a field's value is text")?.to_string(),
            };
            let was = before.get(name).map(|f| f.secret).or_else(|| field["from"].as_str().and_then(|from| before.get(from)).map(|f| f.secret));
            let secret = field["secret"].as_bool().or(was).unwrap_or(false);
            item.fields.insert(name.clone(), Field { value, secret });
        }
    }
    if let Some(hosts) = data["hosts"].as_array() {
        item.hosts = hosts.iter().filter_map(|h| h.as_str().map(str::to_string)).collect();
        ensure!(item.hosts.iter().all(|h| crate::members::valid_host(h)), "a host is a host name, like api.example.com");
    }
    ensure!(!item.hosts.is_empty() || !item.fields.values().any(|f| f.secret), "an item with a secret field is sent to one host or more");
    if let Some(apps) = data["apps"].as_array() {
        item.apps = apps.iter().filter_map(|a| a.as_str()).filter(|a| known(a)).map(str::to_string).collect();
    }
    item.user = user.to_string();
    item.time = time;
    items.insert(id.clone(), item);
    Ok(id)
}

/// After `app` is erased: it's off every item, and the items it made or only it used are gone.
pub fn forget_app(items: &mut BTreeMap<String, Item>, app: &str) {
    items.retain(|_, item| {
        let only = item.apps.len() == 1 && item.apps.contains(app);
        item.apps.remove(app);
        !(only || (item.app.as_deref() == Some(app) && item.apps.is_empty()))
    });
}

/// The oldest item `app` is switched on for that has `field`, with its id.
pub fn find<'a>(items: &'a BTreeMap<String, Item>, app: &str, field: &str) -> Option<(&'a String, &'a Item)> {
    items.iter().filter(|(_, i)| i.apps.contains(app) && i.fields.contains_key(field)).min_by_key(|(_, i)| i.made)
}

/// What a placeholder gives: the field's value, and the hosts it may go to (none for a plain field: anywhere).
pub fn filled(item: &Item, field: &str) -> Option<(Vec<String>, String)> {
    let f = item.fields.get(field)?;
    Some((if f.secret { item.hosts.clone() } else { Vec::new() }, f.value.clone()))
}

/// An item as an app sees it (`ctx.vault.items()`): plain fields' values, and which are secret.
pub fn for_app(id: &str, scope: &str, item: &Item) -> Value {
    let fields: serde_json::Map<String, Value> = item.fields.iter().map(|(n, f)| (n.clone(), if f.secret { json!({ "secret": true }) } else { json!({ "value": f.value }) })).collect();
    json!({ "id": id, "scope": scope, "title": item.title, "type": item.kind, "domain": item.domain, "hosts": item.hosts, "fields": fields, "app": item.app, "time": item.made })
}

/// An item's fields as a person sees them: each name, whether it's secret, and a plain one's value.
pub fn fields_shown(item: &Item) -> Vec<Value> {
    item.fields.iter().map(|(n, f)| if f.secret { json!({ "name": n, "secret": true }) } else { json!({ "name": n, "secret": false, "value": f.value }) }).collect()
}

/// Fills each `{{<item id>:field}}`, `{{field}}` or `{{<group id>/field}}` in `text` with what `lookup` finds for
/// it, as `(hosts, value)`; refuses when a secret field isn't sent to `host`. A placeholder nothing has stays as it is.
pub fn fill(text: &str, host: &str, lookup: impl Fn(&str) -> Option<(Vec<String>, String)>) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push_str(&rest[..start]);
        let name = &after[..end];
        match placeholder(name).then(|| lookup(name)).flatten() {
            Some((hosts, value)) => {
                if !hosts.is_empty() && !hosts.iter().any(|h| h == host) {
                    bail!("{name} is sent only to {}, not {host}", hosts.join(", "));
                }
                out.push_str(&value);
                rest = &after[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Fills every text in `value` as `fill` fills one, for a call to a sidecar (`host` is its name).
pub fn fill_value(value: &mut Value, host: &str, lookup: &impl Fn(&str) -> Option<(Vec<String>, String)>) -> Result<()> {
    match value {
        Value::String(text) if text.contains("{{") => *text = fill(text, host, lookup)?,
        Value::Array(all) => all.iter_mut().try_for_each(|v| fill_value(v, host, lookup))?,
        Value::Object(all) => all.values_mut().try_for_each(|v| fill_value(v, host, lookup))?,
        _ => {}
    }
    Ok(())
}

/// Blots each of `secrets` out of every text in `value`, wherever an answer echoes one back.
pub fn blot(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => {
            for s in secrets.iter().filter(|s| !s.is_empty()) {
                if text.contains(s.as_str()) {
                    *text = text.replace(s.as_str(), "••••");
                }
            }
        }
        Value::Array(all) => all.iter_mut().for_each(|v| blot(v, secrets)),
        Value::Object(all) => all.values_mut().for_each(|v| blot(v, secrets)),
        _ => {}
    }
}

/// Gives each field of an item's change that asks for one (`{ generate: true }`) a new random value, a secret one:
/// a key nobody, not even the app that asked, ever sees.
pub fn generate(data: &mut Value) {
    for field in data["fields"].as_object_mut().into_iter().flat_map(|f| f.values_mut()) {
        if field["generate"] == true {
            *field = json!({ "value": data_encoding::BASE64URL_NOPAD.encode(&rand::random::<[u8; 32]>()), "secret": true });
        }
    }
}

/// Whether `name` could be a placeholder: `field`, `<group id>/field` or `<item id>:field`.
fn placeholder(name: &str) -> bool {
    let field = match (name.split_once('/'), name.split_once(':')) {
        (Some((group, field)), None) if is_id(group) => field,
        (None, Some((item, field))) if is_id(item) => field,
        (None, None) => name,
        _ => return false,
    };
    valid_field(field)
}

/// The Vault is the node's, not an app's: agents list its items (never a secret field's value), set and remove
/// fields, switch apps on for items, and ask the person for one.
#[cfg(not(target_family = "wasm"))]
pub fn tools() -> Vec<Value> {
    use crate::skills::{confirm_input, hints};
    let scope = json!({ "type": "string", "description": "user (yours, the default), global (everyone's), or a group's name or id" });
    let item = json!({ "type": "string", "description": "the item's title or id" });
    let mut web = hints(false, false);
    web["openWorldHint"] = json!(true);
    vec![
        json!({ "name": "vault_list", "description": "The Vault's items you may change: title, type, fields (a secret one never with its value), the sites they're sent to (hosts), the apps switched on for each (apps) and those that want it (wanted_by), and app when an app's sign-in keeps it.", "inputSchema": { "type": "object", "properties": { "scope": scope } }, "annotations": hints(true, false) }),
        json!({ "name": "vault_set", "description": "Set a field of an item, making the item if no item has that title. An app puts {{<item id>:<field>}} in a request and your device fills it in, a secret field only for the item's hosts; nobody reads a secret back. When the person has the value, use vault_request instead: never ask them to paste it into the chat.", "inputSchema": { "type": "object", "required": ["item", "field", "value"], "properties": { "item": item, "field": { "type": "string" }, "value": { "type": "string" }, "secret": { "type": "boolean", "description": "never shown again; true for keys, passwords and tokens" }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "app": { "type": "string", "description": "an app to switch on for it" }, "scope": scope } }, "annotations": hints(false, false) }),
        json!({ "name": "vault_use", "description": "Switch an app on (or off) for an item, so its calls may use the item's fields. Only the person switches agents on (for vault_fetch): ask them with vault_request.", "inputSchema": { "type": "object", "required": ["item", "app"], "properties": { "item": item, "app": { "type": "string" }, "on": { "type": "boolean", "description": "false switches it off" }, "scope": scope } }, "annotations": hints(false, false) }),
        json!({ "name": "vault_remove", "annotations": hints(false, true), "description": "Delete an item, or with field, one of its fields.", "inputSchema": { "type": "object", "required": ["item"], "properties": { "item": item, "field": { "type": "string" }, "scope": scope, "confirm": confirm_input() } } }),
        json!({ "name": "vault_request", "description": "Ask the person for an item an app may use, instead of handling its values yourself: need names what the app declares (its vault), or give title, fields and hosts. Returns a link to give them; it opens a sheet where they pick one of their items or make one (and their Settings lists it). You never see a secret. Tell the person; check vault_status when they say they've done it.", "inputSchema": { "type": "object", "properties": { "app": { "type": "string", "description": "the app that will use it; leave it out for one you use yourself with vault_fetch" }, "need": { "type": "string", "description": "what the app declares it needs, by name, like client for gmail; leave it out and give title, fields and hosts for anything else" }, "title": { "type": "string" }, "fields": { "type": "object", "description": "field name → { secret: true|false }", "additionalProperties": { "type": "object", "properties": { "secret": { "type": "boolean" } } } }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "why": { "type": "string", "description": "one line the person sees: what it's for and where to find it" }, "scope": scope } }, "annotations": hints(false, false) }),
        json!({ "name": "vault_fetch", "description": "Make an HTTP request with your person's Vault items filled in, without seeing them: write {{<field>}} (or {{<item id>:<field>}}) where a value goes, in url, headers or body, and your device fills it in from the items they let agents use, a secret field only for its item's sites. You get { status, headers, body }; a secret the site echoes back is blotted out. To get an item, ask with vault_request, leaving out app.", "inputSchema": { "type": "object", "required": ["url"], "properties": { "url": { "type": "string" }, "method": { "type": "string", "description": "GET by default, POST with a body" }, "headers": { "type": "object", "additionalProperties": { "type": "string" }, "description": "like { \"authorization\": \"Bearer {{token}}\" }" }, "body": { "type": "string" } } }, "annotations": web }),
        json!({ "name": "vault_status", "description": "How a request for an item went: waiting, set, declined or expired, and who answered. Never a value.", "inputSchema": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } }, "annotations": hints(true, false) }),
    ]
}

#[test]
fn fills_fields_only_for_their_hosts() {
    let lookup = |name: &str| match name {
        "token" => Some((vec!["api.example.com".to_string()], "s3cret".to_string())),
        "0123456789abcdef/team" => Some((vec!["api.example.com".to_string()], "shared".to_string())),
        "0123456789abcdef:user" => Some((vec![], "andre".to_string())),
        _ => None,
    };
    assert_eq!(fill("Bearer {{token}}", "api.example.com", lookup).unwrap(), "Bearer s3cret");
    assert_eq!(fill("{{token}}:{{0123456789abcdef/team}}", "api.example.com", lookup).unwrap(), "s3cret:shared");
    assert!(fill("Bearer {{token}}", "evil.example.com", lookup).is_err(), "never to another host");
    assert_eq!(fill("{{0123456789abcdef:user}}", "evil.example.com", lookup).unwrap(), "andre", "a plain field goes anywhere");
    assert_eq!(fill("{{nope}} {{ not one }} {{", "evil.example.com", lookup).unwrap(), "{{nope}} {{ not one }} {{");
}
