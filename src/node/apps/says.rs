//! What the apps' routes say of themselves (src/says.rs).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::says::{Op, Said, Who};

pub fn listed() -> Vec<Said> {
    vec![
        ("/api/apps", Who::Clients, Op::get("Every app the caller can see, with its tools where it's added for them").answers_as(json!({
            "type": "array",
            "items": {
                "type": "object",
                "required": ["name", "owner", "deploys", "added"],
                "properties": {
                    "name": { "type": "string" },
                    "title": { "type": "string" },
                    "owner": { "type": "string", "description": "user:<id>, group:<id> or admins" },
                    "from": { "type": ["string", "null"], "description": "the app it was forked from" },
                    "forks": { "type": "array", "items": { "type": "string" } },
                    "deploys": { "type": "integer" },
                    "bundle": { "type": ["string", "null"], "description": "its latest deploy" },
                    "added": { "type": "array", "items": { "type": "string" }, "description": "the scopes the caller sees it added in" },
                    "deleted": { "type": "object", "description": "{ time, by, yours }, once deleted", "additionalProperties": true },
                    "tools": { "type": "array", "items": crate::openapi::tool_spec(), "description": "where it's added for the caller" },
                    "widgets": { "type": "array", "items": { "type": "object" } },
                    "screens": { "type": "array", "items": { "type": "object" } },
                    "schedules": { "type": "array", "items": { "type": "object" } }
                },
                "additionalProperties": true
            }
        }))),
        ("/api/labs", Who::Clients, Op::get("Every lab, for an admin").answers_as(json!({
            "type": "array",
            "items": { "type": "object", "required": ["title", "description", "on", "setting"], "properties": { "title": { "type": "string" }, "description": { "type": "string" }, "on": { "type": "boolean" }, "setting": { "type": "string" } }, "additionalProperties": true }
        }))),
        ("/api/deploy", Who::Clients, Op::post("Deploy an app, making it first if there's none by that name").body::<Deploy>().answers_as(json!({
            "type": "object",
            "required": ["bundle", "made", "here"],
            "properties": {
                "bundle": { "type": "string", "description": "the deploy's hash" },
                "made": { "type": "boolean", "description": "whether the app is new" },
                "before": { "type": ["string", "null"], "description": "the deploy it replaced" },
                "here": { "type": "boolean", "description": "whether this device runs it yet" },
                "tools": { "type": "array", "items": { "type": "string" }, "description": "its tools, once this device runs it" },
                "lacking": { "type": "array", "items": { "type": "string" }, "description": "those that give no title or no output" },
                "failed": { "type": "string", "description": "why it doesn't start" }
            }
        }))),
        ("/api/arrange", Who::Clients, Op::post("Arrange one of the caller's pages").body::<Arrange>().done()),
    ]
}

/// An app's `bundle`, or its `files` (each file's path → its text, built here as `one deploy` builds a folder), to
/// deploy, making the app first if there's none by that name, owned by `owner` (as `owner` reads it) or the caller.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Deploy {
    pub app: String,
    pub bundle: Option<String>,
    pub files: Option<serde_json::Map<String, Value>>,
    pub owner: Option<String>,
    #[serde(default)]
    pub wait: bool,
}

/// How the local user arranges one of their pages (`pages` with their own: the order of their groups' pages,
/// `{ order, hidden }`).
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Arrange {
    pub page: String,
    #[serde(default)]
    pub items: Value,
    #[serde(default)]
    pub hidden: Value,
    #[serde(default)]
    pub pages: Value,
}
