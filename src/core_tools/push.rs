//! Push's tools: notifications to the user's own devices, for agents (apps push with `ctx.notify`).

use serde_json::{Value, json};

use super::{Feature, Request, Tool, as_given};
use crate::config::YOURS;

pub fn feature() -> Feature {
    Feature {
        name: "push",
        title: "Push",
        description: "Sends notifications to your devices".into(),
        guide: "Push sends a notification to every device of yours that gets them: push_send with a title and a body. An app pushes with ctx.notify, and tapping its push opens its screen. Each one is sealed to the device it's for, so nothing on the way reads it. It says how many devices it went to, never whether anyone saw it.".into(),
        tools: tools(),
        #[cfg(feature = "node")]
        steps: super::no_steps,
    }
}

fn tools() -> Vec<Tool> {
    let none = json!({ "type": "object", "properties": {} });
    vec![
        Tool::asks(json!({ "name": "send", "description": "Push a notification to your devices: { title, body?, screen?, params? }; from an app, tapping it opens that app's screen (main unless given). Says how many devices it went to.",
            "input": { "type": "object", "required": ["title"], "properties": {
                "title": { "type": "string", "description": "one line" },
                "body": { "type": "string", "description": "the text under it" },
                "app": { "type": "string", "description": "the app it opens, when no app sends it" },
                "screen": { "type": "string", "description": "the screen it opens" },
                "params": { "type": "object", "description": "that screen's params" },
                "badge": { "type": "integer", "description": "the number on the app's icon; 0 clears it" },
            } } }), |input, _| Ok(Request::post("/v1/push/send", Value::Object(input.clone()))), as_given),
        Tool::asks(json!({ "name": "devices", "reads": true, "description": "Your devices that get notifications.", "input": none }), |_, _| Ok(Request::get("/v1/push/devices")), as_given),
        Tool::asks(json!({ "name": "status", "reads": true, "description": format!("Which push services {YOURS} sends through: apns, fcm, wns, webpush."), "input": none }), |_, _| Ok(Request::get("/v1/push/status")), as_given),
    ]
}
