//! Widgets' tool, its own route: what it takes and answers. What serves it is the runtime's (widgets/runtime/src/tools.rs);
//! it's the user's widgets as text, for an agent, or a chat that can't draw them (api/mcp/src/mcp/apps.rs draws them in one that can,
//! from the same answer).

use serde_json::json;

use ::api::Feature;

pub fn feature() -> Feature {
    Feature {
        name: "widgets",
        title: "Widgets",
        description: "Your widgets, as text".into(),
        guide: String::new(),
        tools: vec![json!({
                "name": "show", "title": "Show the widgets", "reads": true,
                "description": "Show the user's widgets inline, drawn as their apps draw them (or as text where a chat can't draw them): all of them, one app's, or one. Use it when they ask how things stand at a glance. One that says it's still drawing took too long beside the rest: ask for it alone, with its app and widget.",
                "input": { "type": "object", "properties": {
                    "app": { "type": "string", "description": "only this app's widgets" },
                    "widget": { "type": "string", "description": "only the widget of this name" },
                    "scope": { "type": "string", "description": "whose page: user (theirs), group:<id> or global; every page by default" },
                } },
                "output": { "type": "object", "required": ["said", "widgets"], "properties": {
                    "said": { "type": "string", "description": "the widgets, in words" },
                    "widgets": { "type": "array", "items": { "type": "object" }, "description": "each as `GET /api/widgets` gives it, to draw" },
                } },
            })],
    }
}
