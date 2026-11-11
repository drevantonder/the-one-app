//! Notifications' tools, each its own route: notifications to the user's own devices, for agents (apps send with
//! `ctx.notify`). What serves them is the runtime's (notifications/runtime/src/tools.rs).

use serde_json::json;

use ::custom_build::config::YOURS;
use ::api::Feature;

pub fn feature() -> Feature {
    Feature {
        name: "notifications",
        title: "Notifications",
        description: "Sends notifications to your devices".into(),
        guide: "Notifications go to every device of yours that gets them: notifications_send with a title and a body. An app sends one with ctx.notify, and tapping it opens its screen. Each one is sealed to the device it's for, so nothing on the way reads it. It says how many devices it went to, never whether anyone saw it.".into(),
        tools: tools(),
    }
}

fn tools() -> Vec<serde_json::Value> {
    let none = json!({ "type": "object", "properties": {} });
    vec![
        json!({
                "name": "send", "title": "Send a notification",
                "description": "Send a notification to your devices: { title, body?, screen?, params? }; from an app, tapping it opens that app's screen (main unless given). Says how many devices it went to.",
                "input": { "type": "object", "required": ["title"], "properties": {
                    "title": { "type": "string", "description": "one line" },
                    "body": { "type": "string", "description": "the text under it" },
                    "app": { "type": "string", "description": "the app it opens, when no app sends it" },
                    "screen": { "type": "string", "description": "the screen it opens" },
                    "params": { "type": "object", "description": "that screen's params" },
                } },
                "output": { "type": "object", "required": ["sent", "devices"], "properties": {
                    "sent": { "type": "integer", "description": "how many devices it went to" }, "devices": { "type": "integer", "description": "how many of yours get notifications" },
                    "why": { "type": "string", "description": "what stopped it, when it didn't go to all" },
                } },
            }),
        json!({
                "name": "devices", "title": "Your devices that get notifications", "reads": true,
                "description": "Your devices that get notifications.",
                "input": none,
                "output": { "type": "array", "items": { "type": "object", "required": ["name", "via"], "properties": {
                    "name": { "type": "string" }, "via": { "type": "string", "description": "apns, fcm, wns or webpush" }, "since": { "type": "integer", "description": "when it registered, in ms since 1970" },
                } } },
            }),
        json!({
                "name": "status", "title": "Which push services notifications go through", "reads": true,
                "description": format!("Which push services {YOURS} sends through: apns, fcm, wns, webpush."),
                "input": none,
                "output": { "type": "object", "required": ["via"], "properties": { "via": { "type": "array", "items": { "type": "string" } } } },
            }),
    ]
}
