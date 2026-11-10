//! What the widgets' and badges' routes say of themselves (src/says.rs).

use serde::Deserialize;
use serde_json::json;

use crate::says::{Op, Said, Who};

pub fn listed() -> Vec<Said> {
    vec![
        ("/api/widgets", Who::Clients, Op::get("The caller's widgets, drawn by their apps").query::<WidgetsAsked>().answers_as(json!({ "type": "array", "items": { "type": "object", "required": ["app", "widget"], "properties": { "app": { "type": "string" }, "widget": { "type": "string" }, "late": { "type": "boolean", "description": "took too long to draw beside the rest: ask for it alone, with kind" } }, "additionalProperties": true } }))),
        ("/api/badges", Who::Clients, Op::get("The caller's badges").query::<WidgetsQuery>().answers_as(json!({ "type": "object", "description": "each app's name → its badge, a number above 0; an app with none is left out", "additionalProperties": { "type": "number" } }))),
    ]
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WidgetsQuery {
    pub scope: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WidgetsAsked {
    pub scope: Option<String>,
    /// `<app>.<widget>`: that widget alone, however long it takes.
    pub kind: Option<String>,
}
