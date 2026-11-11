//! The Vault's tools, each its own route: what it takes and answers. What serves them is the runtime's
//! (vault/runtime/src/tools.rs). `grant` changes who can do what, so the caller's client answers it (`settings::answered`), and
//! its route is only this machine's own programs', whose node answers it as its own device.

use serde_json::{Value, json};

use ::api::Feature;

const GUIDE: &str = "The Vault keeps the user's keys, sign-ins and accounts as items, each a page's (theirs, a group's, everyone's): fields (secret ones never read back), the sites they're sent to, and the apps switched on for it. Never ask the user to paste a secret (an API key, a password, a token) into the chat, and never paste one yourself. To keep one, call vault_request and give them its link: they pick one of their items or type a new one into a sheet on their own device, and you never see it. To use one yourself, ask leaving out app, then call vault_fetch with {{<field>}} where it goes: a machine that opens the Vault fills it in, only for the item's sites, and you get the answer. If someone pastes a secret anyway, keep it with vault_set and don't repeat it. vault_list shows what's there.";

pub fn feature() -> Feature {
    Feature {
        name: "vault",
        title: "Vault",
        description: "Keys, sign-ins and accounts, used without being seen".into(),
        guide: GUIDE.into(),
        tools: tools(),
    }
}

/// An answer that's only words for whoever asked.
fn said(about: &str) -> Value {
    json!({ "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": about } } })
}

/// The Vault is the node's, not an app's: agents list its items (never a secret field's value), set and remove
/// fields, switch apps on for items, and ask the user for one.
fn tools() -> Vec<serde_json::Value> {
    let scope = json!({ "type": "string", "description": "user (yours, the default), global (everyone's), or a group's name or id" });
    let item = json!({ "type": "string", "description": "the item's title or id" });
    let strings = json!({ "type": "array", "items": { "type": "string" } });
    vec![
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
        json!({
                "name": "set", "title": "Keep a secret in the Vault",
                "description": "Keep a key, token or password someone pasted (or any field) in an item, making the item if no item has that title. An app puts {{<item id>:<field>}} in a request and a machine that opens the Vault fills it in, a secret field only for the item's hosts; nobody reads a secret back, not you either. When the user has the value, use vault_request instead: never ask them to paste it into the chat.",
                "input": { "type": "object", "required": ["item", "field", "value"], "properties": { "item": item, "field": { "type": "string", "description": "the field's name, like token or password" }, "value": { "type": "string", "description": "the field's value" }, "secret": { "type": "boolean", "description": "never shown again; true for keys, passwords and tokens" }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "app": { "type": "string", "description": "an app to switch on for it" }, "scope": scope } },
                "output": said("what was set, and how agents use it"),
            }),
        json!({
                "name": "use", "title": "Switch an app on for an item",
                "description": "Switch an app on (or off) for an item, so its calls may use the item's fields. Only the user switches agents on (for vault_fetch): ask them with vault_request.",
                "input": { "type": "object", "required": ["item", "app"], "properties": { "item": item, "app": { "type": "string", "description": "the app's name, like gmail" }, "on": { "type": "boolean", "description": "false switches it off" }, "scope": scope } },
                "output": said("what's switched on or off"),
            }),
        json!({
                "name": "grant", "title": "Let a machine open a Vault",
                "description": format!("Let a machine of {} open a Vault's secrets and fill them into requests (or, with on false, no longer). Only the Vault's admins say which: the user for their own, a group's admins, the network's for everyone's. A Vault no machine opens keeps no secret, so grant before you set one. Nobody else reads a secret back, not even on their own device.", ::custom_build::config::YOURS),
                "input": { "type": "object", "required": ["device"], "properties": { "device": { "type": "string", "description": "the machine's name" }, "on": { "type": "boolean", "description": "false: it no longer opens it" }, "scope": scope } },
                "output": said("which machine opens the Vault now"),
            }),
        json!({
                "name": "remove", "title": "Delete an item or a field", "confirm": true, "words": ["item", "field"],
                "description": "Delete an item, or with field, one of its fields.",
                "input": { "type": "object", "required": ["item"], "properties": { "item": item, "field": { "type": "string", "description": "only this field, by name; the whole item when not given" }, "scope": scope } },
                "output": said("what was deleted"),
            }),
        json!({
                "name": "request", "title": "Ask the user for a secret", "words": ["app"],
                "description": "Ask the user for a key, token, password or sign-in an app or agents may use, instead of handling its values yourself: need names what the app declares (its vault), or give title, fields and hosts. Returns a link to give them; it opens a sheet where they pick one of their items or make one (and their Settings lists it). You never see a secret. Tell the user; check vault_status when they say they've done it.",
                "input": { "type": "object", "properties": { "app": { "type": "string", "description": "the app that will use it; leave it out for one you use yourself with vault_fetch" }, "need": { "type": "string", "description": "what the app declares it needs, by name, like client for gmail; leave it out and give title, fields and hosts for anything else" }, "title": { "type": "string", "description": "the item's title, for one no app declares" }, "fields": { "type": "object", "description": "field name → { secret: true|false }", "additionalProperties": { "type": "object", "properties": { "secret": { "type": "boolean" } } } }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "why": { "type": "string", "description": "one line the user sees: what it's for and where to find it" }, "scope": scope } },
                "output": { "type": "object", "required": ["id", "scope", "link", "said"], "properties": {
                    "id": { "type": "string", "description": "the request's, for vault_status" }, "scope": { "type": "string" },
                    "link": { "type": "string", "description": "where the user answers it" }, "said": { "type": "string", "description": "what to tell the user" },
                } },
            }),
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
        json!({
                "name": "status", "title": "How a request for an item went", "reads": true,
                "description": "How a request for an item went: waiting, set, declined or expired, and who answered. Never a value.",
                "input": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "description": "the request's id, from vault_request" } } },
                "output": { "type": "object", "required": ["state"], "properties": {
                    "state": { "type": "string", "description": "waiting, expired, or how it was answered" }, "app": { "type": "string" }, "title": { "type": "string" },
                    "answered": { "type": "object", "properties": { "by": { "type": "string" }, "time": { "type": "integer" } }, "description": "who answered it, and when, in ms since 1970" },
                } },
            }),
    ]
}
