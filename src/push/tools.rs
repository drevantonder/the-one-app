//! Push's tools, each its own route (`api::Runs`): notifications to the user's own devices, for agents
//! (apps push with `ctx.notify`).

use serde_json::json;

use ::custom_build::config::YOURS;
use ::api::Feature;

pub fn feature() -> Feature {
    Feature {
        name: "push",
        title: "Push",
        description: "Sends notifications to your devices".into(),
        guide: "Push sends a notification to every device of yours that gets them: push_send with a title and a body. An app pushes with ctx.notify, and tapping its push opens its screen. Each one is sealed to the device it's for, so nothing on the way reads it. It says how many devices it went to, never whether anyone saw it.".into(),
        tools: tools(),
    }
}

fn tools() -> Vec<serde_json::Value> {
    let none = json!({ "type": "object", "properties": {} });
    vec![
        json!({
                "name": "send", "title": "Push a notification",
                "description": "Push a notification to your devices: { title, body?, screen?, params? }; from an app, tapping it opens that app's screen (main unless given). Says how many devices it went to.",
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
                "name": "status", "title": "Which push services are in use", "reads": true,
                "description": format!("Which push services {YOURS} sends through: apns, fcm, wns, webpush."),
                "input": none,
                "output": { "type": "object", "required": ["via"], "properties": { "via": { "type": "array", "items": { "type": "string" } } } },
            }),
    ]
}

/// What serves each tool, in this runtime.
#[cfg(feature = "node")]
pub fn runs() -> ::api::Runs {
    use crate::node::runtime::node_serves;
    ::api::Runs {
        tools: vec![
            ("send", node_serves!(serve::send)),
            ("devices", node_serves!(serve::devices)),
            ("status", node_serves!(serve::status)),
        ],
        ..::api::Runs::named("push")
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value};

    use crate::node::Node;
    use ::api::Caller;

    /// A note to the caller's devices: → how many it went to, and how many of theirs get pushes.
    pub async fn send(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let send: crate::push::Send = serde_json::from_value(Value::Object(input))?;
        node.push_note(&me.user, &send.note("")?).await
    }

    pub async fn devices(node: Node, me: Caller, _: Map<String, Value>) -> Result<Value> {
        node.push_devices(&me.user).await
    }

    pub async fn status(node: Node, _: Caller, _: Map<String, Value>) -> Result<Value> {
        node.push_status().await
    }
}
