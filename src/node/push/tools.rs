//! Push's tools: notifications to the person's own devices, for agents (apps push with `ctx.notify`).

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use crate::config::YOURS;
use crate::core_tools::{Asked, Feature};
use crate::skills::hints;

pub fn feature() -> Feature {
    Feature {
        name: "push",
        title: "Push",
        description: "Sends notifications to your devices".into(),
        guide: "Push sends a notification to every device of yours that gets them: push_send with a title and a body. An app pushes with ctx.notify, and tapping its push opens its screen. Each one is sealed to the device it's for, so nothing on the way reads it. It says how many devices it went to, never whether anyone saw it.".into(),
        tools: tools(),
        elsewhere: true,
        run,
    }
}

fn tools() -> Vec<Value> {
    let none = json!({ "type": "object", "properties": {} });
    vec![
        json!({ "name": "push_send", "annotations": hints(false, false), "description": "Push a notification to your devices: { title, body?, screen?, params? }; from an app, tapping it opens that app's screen (main unless given). Says how many devices it went to.",
            "inputSchema": { "type": "object", "required": ["title"], "properties": {
                "title": { "type": "string", "description": "one line" },
                "body": { "type": "string", "description": "the text under it" },
                "app": { "type": "string", "description": "the app it opens, when no app sends it" },
                "screen": { "type": "string", "description": "the screen it opens" },
                "params": { "type": "object", "description": "that screen's params" },
                "badge": { "type": "integer", "description": "the number on the app's icon; 0 clears it" },
            } } }),
        json!({ "name": "push_devices", "annotations": hints(true, false), "description": "Your devices that get notifications.", "inputSchema": none }),
        json!({ "name": "push_status", "annotations": hints(true, false), "description": format!("Which push services {YOURS} sends through: apns, fcm, wns, webpush."), "inputSchema": none }),
    ]
}

fn run(tool: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value> {
    match tool {
        "send" => asked.node.post("/v1/push/send", Value::Object(input.clone())),
        "devices" => asked.node.get("/v1/push/devices"),
        "status" => asked.node.get("/v1/push/status"),
        other => bail!("no tool push_{other}"),
    }
}
