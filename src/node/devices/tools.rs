//! Devices' tools: letting a device in, taking one out, inviting someone, and where an app's schedules run.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::config::YOURS;
use crate::core_tools::{Asked, Feature, text};

pub fn feature() -> Feature {
    let guide = format!("A new device signs in, then shows a code of 8 letters and digits. devices_approve with that code, run on one of the person's own devices, lets it in once they say yes: it gets the keys to what they use. devices_invite makes a link for someone new to join; their first device is approved the same way, from any of the inviting admin's devices. devices_remove takes a device out, or a person with all of theirs, and what they could open gets new keys. Someone who lost every device is let back in with devices_approve's recovery_key: a file holding {YOURS}'s recovery key, never the key in the chat. An app's schedules run only on a device chosen for them: devices_run_schedules picks one, which then holds that app's keys alone, and devices_stop_schedules stops them.");
    Feature {
        name: "devices",
        title: "Devices",
        description: "Set up a new laptop, phone or computer and let it in, take one out, run an app's schedules on one, and invite someone".into(),
        guide,
        tools: tools(),
        // This machine's person's to run, as its own: never someone signed in from elsewhere.
        elsewhere: false,
        run,
    }
}

fn tools() -> Vec<Value> {
    let confirm = json!({ "type": "boolean", "description": "true once the person has said yes to this very device" });
    vec![
        json!({ "name": "requests", "reads": true, "description": "Devices waiting to join that you may approve: your own new devices, and the first device of each person you invited.", "input": { "type": "object", "properties": {} } }),
        json!({ "name": "approve", "description": "Approve the new device showing this code. Without confirm, says whose device it is; with confirm: true, lets it in. With recovery_key, lets back in someone who lost all their devices.", "input": { "type": "object", "properties": { "code": { "type": "string", "description": "the 8 letters and digits the new device shows" }, "confirm": confirm, "recovery_key": { "type": "string", "description": format!("the path of a file on this machine holding {YOURS}'s recovery key (- reads it from stdin on the command line); never the key itself") } }, "required": ["code"] } }),
        json!({ "name": "remove", "confirm": true, "description": "Take a device out, by its name, or a person, with all their devices: they open nothing made after. Their own data stays.", "input": { "type": "object", "properties": { "device": { "type": "string", "description": "a device's name" }, "user": { "type": "string", "description": "a person's name, to remove them" } } } }),
        json!({ "name": "run_schedules", "description": "Run an app's schedules on one of your devices: the device holds only that app's keys in that place, for 30 days and longer as you use the app. Settings › Devices lists them under App Schedules, with Stop.", "input": { "type": "object", "properties": { "app": { "type": "string", "description": "the app's name" }, "device": { "type": "string", "description": "the device that runs them, by name" }, "scope": { "type": "string", "description": "where: user (yours, the default), a group by its name, or global" }, "days": { "type": "integer", "description": "how long, at most 30" } }, "required": ["app", "device"] } }),
        json!({ "name": "stop_schedules", "description": "Stop an app's schedules on a device: they no longer run there, and it no longer opens that app's data.", "input": { "type": "object", "properties": { "app": { "type": "string" }, "device": { "type": "string" }, "scope": { "type": "string", "description": "user (the default), a group by its name, or global" } }, "required": ["app", "device"] } }),
        json!({ "name": "invite", "description": "A link for someone new to join, once, for a few days. Give it to them yourself; it sends nothing.", "input": { "type": "object", "properties": { "name": { "type": "string", "description": "their name" }, "admin": { "type": "boolean", "description": "make them an admin" } }, "required": ["name"] } }),
    ]
}

fn run(tool: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value> {
    let node = asked.node;
    let text = |k: &str| text(input, k);
    match tool {
        "requests" => node.get("/v1/devices/requests"),
        "approve" => {
            let mut body = json!({ "code": text("code")?, "confirm": asked.confirmed });
            if let Some(at) = input.get("recovery_key").and_then(Value::as_str) {
                body["recovery"] = json!(recovery_key_from(at, asked.reader)?);
            }
            node.post("/v1/devices/approve", body).map(|r| json!(approved_said(&r)))
        }
        "remove" => {
            let what = match (input.get("device").and_then(Value::as_str), input.get("user").and_then(Value::as_str)) {
                (Some(device), None) => json!({ "key": device }),
                (None, Some(user)) => json!({ "user": user }),
                _ => bail!("give device (one device) or user (a person, with all their devices)"),
            };
            node.post("/v1/remove", what).map(|r| json!(removed_said(&r)))
        }
        "run_schedules" | "stop_schedules" => {
            let scope = input.get("scope").and_then(Value::as_str).unwrap_or("user");
            let days = if tool == "stop_schedules" { json!(0) } else { input.get("days").cloned().unwrap_or(Value::Null) };
            node.post("/v1/app-schedules", json!({ "app": text("app")?, "device": text("device")?, "scope": scope, "days": days })).map(|r| r["said"].clone())
        }
        "invite" => node.post("/v1/invite-link", json!({ "name": text("name")?, "admin": input.get("admin") == Some(&json!(true)) })).map(|r| json!(invited_said(text("name").unwrap_or(""), &r))),
        other => bail!("no tool devices_{other}"),
    }
}

/// The network's recovery key, from the file at `at` on this machine, or stdin for `-` on the command line: never
/// typed into a chat, and never printed.
pub fn recovery_key_from(at: &str, reader: crate::tools::Reader) -> Result<String> {
    let looks_like_key = |t: &str| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit());
    anyhow::ensure!(!looks_like_key(at.trim()), "give the file the recovery key is in, never the key itself");
    let text = match at {
        "-" => {
            anyhow::ensure!(reader == crate::tools::Reader::Cli, "- reads the recovery key from stdin on the command line: give a file's path instead");
            std::io::read_to_string(std::io::stdin())?
        }
        path => std::fs::read_to_string(path).with_context(|| format!("couldn't read {path}"))?,
    };
    let key = text.trim().to_ascii_lowercase();
    anyhow::ensure!(looks_like_key(&key), "that file doesn't hold a recovery key: 64 hex characters");
    Ok(key)
}

fn approved_said(r: &Value) -> String {
    let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
    if let Some(service) = r["service"].as_str() {
        return match r.get("approved") {
            Some(_) => format!("{service} on {} is in: it answers your devices' calls.", s("device")),
            None => format!("{service} on {} wants to be yours. Approve it only if that's the code it shows.", s("device")),
        };
    }
    if r.get("sealed").is_none() && r["recovery"] == true {
        return format!("{} is {}'s device. Approve it with the recovery key only if they lost all their devices and say it's theirs.", s("device"), s("person"));
    }
    if r.get("sealed").is_none() {
        let new = if r["new"] == true { ", who you invited" } else { "" };
        return format!("{} is {}'s device{new}. Approve it only if they say it's theirs.", s("device"), s("person"));
    }
    let mut out = format!("{} is in.", s("device"));
    if r["recovery"] == true {
        out = format!("{} is in, let in with the recovery key: {} can open what they could before.", s("device"), s("person"));
    }
    if let Some(relay) = r["relay"].as_str() {
        out.push_str(&format!(" The network didn't take it yet: {relay}."));
    }
    for n in r["not_sealed"].as_array().into_iter().flatten() {
        out.push_str(&format!(" It wasn't given {}'s keys: {}.", n["scope"].as_str().unwrap_or(""), n["why"].as_str().unwrap_or("")));
    }
    out
}

fn removed_said(r: &Value) -> String {
    let mut out = format!("Removed {}.", r["removed"].as_str().unwrap_or("it"));
    let left: Vec<&str> = r["not_renewed"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !left.is_empty() {
        out.push_str(&format!(" New keys for {} come from one of their own devices, or their admins'.", left.join(", ")));
    }
    out
}

pub fn invited_said(name: &str, r: &Value) -> String {
    let days = crate::sign_in_proof::INVITE_DAYS;
    format!("Send {name} this link. It works once, for {days} days: opening it, they join. Then approve their device with its code.\n\n  {}", r["link"].as_str().unwrap_or(""))
}
