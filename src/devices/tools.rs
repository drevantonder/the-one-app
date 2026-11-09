//! Devices' tools, each its own route (`core_tools::Runs::Route`). They change who can do what, so the caller's client
//! answers them (`settings::answered`), and each route is only this machine's own programs', whose node answers it as
//! its own device.

use serde_json::json;

use crate::config::YOURS;
use crate::core_tools::{Feature, Tool, serves};

pub fn feature() -> Feature {
    let guide = format!("A new device signs in, then shows a code of 8 letters and digits. devices_approve with that code, run on one of the user's own devices, lets it in once they say yes: it gets the keys to what they use. devices_invite makes a link for someone new to join; their first device is approved the same way, on the device that made the link. devices_remove takes a device out, or a user with all of theirs, and what they could open gets new keys. Someone who lost every device is let back in with devices_approve's recovery_key: a file holding {YOURS}'s recovery key, never the key in the chat. An app's schedules run only on a device chosen for them: devices_run_schedules picks one, which then holds that app's keys alone, and devices_stop_schedules stops them.");
    Feature {
        name: "devices",
        title: "Devices",
        description: "Set up a new laptop, phone or computer and let it in, take one out, run an app's schedules on one, and invite someone".into(),
        guide,
        tools: tools(),
    }
}

/// An answer that's words for whoever asked, and may hold more for a screen.
fn said(about: &str) -> serde_json::Value {
    json!({ "type": "object", "required": ["said"], "additionalProperties": true, "properties": { "said": { "type": "string", "description": about } } })
}

fn tools() -> Vec<Tool> {
    vec![
        Tool::route(
            json!({
                "name": "requests", "title": "Devices waiting to join", "reads": true,
                "description": "Devices waiting to join that you may approve: your own new devices, and the first device of each user you invited.",
                "input": { "type": "object", "properties": {} },
                "output": { "type": "array", "items": { "type": "object", "required": ["device", "user", "code"], "properties": {
                    "device": { "type": "string" }, "user": { "type": "string" }, "code": { "type": "string", "description": "the 8 letters and digits it shows" },
                    "yours": { "type": "boolean", "description": "one of your own" }, "new": { "type": "boolean", "description": "the first of someone you invited" },
                } } },
            }),
            serves!(serve::requests),
        ),
        Tool::route(
            json!({
                "name": "approve", "title": "Approve a new device",
                "description": "Approve the new device showing this code. Without confirm, says whose device it is; with confirm: true, lets it in. With recovery_key, lets back in someone who lost all their devices.",
                "input": { "type": "object", "required": ["code"], "properties": {
                    "code": { "type": "string", "description": "the 8 letters and digits the new device shows" },
                    "confirm": { "type": "boolean", "description": "true once the user has said yes to this very device" },
                    "recovery_key": { "type": "string", "description": format!("the absolute path of a file on this machine holding {YOURS}'s recovery key; never the key itself") },
                } },
                "output": said("whose device it is, or that it's in"),
            }),
            serves!(serve::approve),
        ),
        Tool::route(
            json!({
                "name": "remove", "title": "Take a device or a user out", "confirm": true,
                "description": "Take a device out, by its name, or a user, with all their devices: they open nothing made after. Their own data stays.",
                "input": { "type": "object", "properties": { "device": { "type": "string", "description": "a device's name" }, "user": { "type": "string", "description": "a user's name, to remove them" } } },
                "output": said("what was taken out"),
            }),
            serves!(serve::remove),
        ),
        Tool::route(
            json!({
                "name": "run_schedules", "title": "Run an app's schedules on a device",
                "description": "Run an app's schedules on one of your devices: the device holds only that app's keys in that place, for 30 days and longer as you use the app. Settings › Devices lists them under App Schedules, with Stop.",
                "input": { "type": "object", "required": ["app", "device"], "properties": {
                    "app": { "type": "string", "description": "the app's name" },
                    "device": { "type": "string", "description": "the device that runs them, by name" },
                    "scope": { "type": "string", "description": "where: user (yours, the default), a group by its name, or global" },
                    "days": { "type": "integer", "description": "how long, at most 30" },
                } },
                "output": said("which device runs them now, and for how long"),
            }),
            serves!(serve::run_schedules),
        ),
        Tool::route(
            json!({
                "name": "stop_schedules", "title": "Stop an app's schedules on a device",
                "description": "Stop an app's schedules on a device: they no longer run there, and it no longer opens that app's data.",
                "input": { "type": "object", "required": ["app", "device"], "properties": {
                    "app": { "type": "string" },
                    "device": { "type": "string" },
                    "scope": { "type": "string", "description": "user (the default), a group by its name, or global" },
                } },
                "output": said("which device no longer runs them"),
            }),
            serves!(serve::stop_schedules),
        ),
        Tool::route(
            json!({
                "name": "invite", "title": "Invite someone",
                "description": "A link for someone new to join, once, for a few days. Give it to them yourself; it sends nothing.",
                "input": { "type": "object", "required": ["name"], "properties": {
                    "name": { "type": "string", "description": "their name" },
                    "admin": { "type": "boolean", "description": "make them an admin" },
                    "user": { "type": "string", "description": "someone in the network already, by id, whose new device this lets in; omit for someone new" },
                    "email": { "type": "string", "description": "where users sign in with Google: the Google account they sign up with" },
                } },
                "output": said("the link to give them, and what happens when they open it"),
            }),
            serves!(serve::invite),
        ),
    ]
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value};

    use crate::core_tools::text;
    use crate::node::{Caller, Node};

    /// This machine's node answers as its own device (`settings`), signing what changes who can do what.
    async fn as_mine(node: &Node, path: &str, input: &Map<String, Value>) -> Result<Value> {
        Ok(node.answer_as_mine("POST", path, &serde_json::to_vec(input)?).await.map_err(|e| e.0)?.0)
    }

    pub async fn requests(node: Node, _: Caller, input: Map<String, Value>) -> Result<Value> {
        as_mine(&node, "/v1/devices/requests", &input).await
    }

    /// Lets in the device showing `code`, once the user agreed; or with the recovery key in a file here, someone who
    /// lost every device, which only this machine's node does, and never one asked over the network.
    pub async fn approve(node: Node, _: Caller, input: Map<String, Value>) -> Result<Value> {
        let Some(file) = input.get("recovery_key").and_then(Value::as_str) else { return as_mine(&node, "/v1/devices/approve", &input).await };
        let secret = crate::keys::unhex(&crate::keys::recovery_key_in(file)?)?;
        let mut approved = node.approve_with_recovery(text(&input, "code")?, &ed25519_dalek::SigningKey::from_bytes(&secret), input.get("confirm") == Some(&Value::Bool(true))).await?;
        approved["said"] = Value::String(crate::settings::said_approved(&approved));
        Ok(approved)
    }

    pub async fn remove(node: Node, _: Caller, input: Map<String, Value>) -> Result<Value> {
        as_mine(&node, "/v1/devices/remove", &input).await
    }

    pub async fn run_schedules(node: Node, _: Caller, input: Map<String, Value>) -> Result<Value> {
        as_mine(&node, "/v1/devices/run_schedules", &input).await
    }

    pub async fn stop_schedules(node: Node, _: Caller, input: Map<String, Value>) -> Result<Value> {
        as_mine(&node, "/v1/devices/stop_schedules", &input).await
    }

    pub async fn invite(node: Node, _: Caller, input: Map<String, Value>) -> Result<Value> {
        as_mine(&node, "/v1/devices/invite", &input).await
    }
}
