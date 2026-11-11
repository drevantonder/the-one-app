//! What the deploy's route says of itself (api/features/src/says.rs).

use serde::Deserialize;
use serde_json::{Value, json};

use ::api::says::Op;
use ::api::says::Said;
use ::api::says::Who;

pub fn listed() -> Vec<Said> {
    vec![
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
