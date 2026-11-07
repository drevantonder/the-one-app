//! Adding an app and removing it, the two tools of the core that are the one way to do either: the command line's
//! `add` and `remove` ask the same route (`/v1/add`), and an app's screens through its own `ctx.add`.

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use super::{Asked, Feature, Request, Tool};

const GUIDE: &str = "add puts an app, and the apps it uses, on a page: yours (scope user), a group you're an admin of (by its name) or everyone's (scope global, admins only). An app with a service runs it on one of your nodes it can run on (node), or the Cloudflare node for everyone's or a group's; asked when there's a choice. remove takes it off a page, and what it keeps there goes with it. The App Store (apps_list) lists every app and where it's added.";

fn feature(name: &'static str, title: &'static str, description: &str, tool: Tool) -> Feature {
    Feature {
        name,
        title,
        description: description.into(),
        guide: GUIDE.into(),
        tools: vec![tool],
        #[cfg(feature = "node")]
        steps: super::no_steps,
    }
}

pub fn add_feature() -> Feature {
    let place = json!({ "type": "string", "description": "user (you), global (everyone) or a group's name" });
    let input = json!({ "type": "object", "required": ["app", "scope"], "properties": {
        "app": { "type": "string", "description": "an app's name, from apps_list, like gmail" },
        "scope": place,
        "node": { "type": "string", "description": "the node that runs its service: one of yours by name, or cloudflare, as apps_devices lists them" },
    } });
    feature("add", "Add an app", "Add an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only)", Tool::asks(json!({ "name": "add", "description": "Add an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only). It shows on that place's page. An app with a service runs it on node, one of your nodes it can run on (apps_devices lists them), or for everyone's or a group's, the Cloudflare node; asked when there's a choice.", "input": input }), add, |input, _| said("Added", input)))
}

pub fn remove_feature() -> Feature {
    let input = json!({ "type": "object", "required": ["app", "scope"], "properties": {
        "app": { "type": "string", "description": "an app's name, from apps_list, like gmail" },
        "scope": { "type": "string", "description": "user (you), global (everyone) or a group's name" },
    } });
    feature("remove", "Remove an app", "Remove an app from a place", Tool::asks(json!({ "name": "remove", "confirm": true, "description": "Remove an app from a place: user (you), a group (by its name) or global (everyone). What it keeps there goes with it.", "input": input }), remove, |input, _| said("Removed", input)))
}

/// A text input that isn't empty, or what to give.
fn text<'a>(input: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    input.get(key).and_then(Value::as_str).filter(|v| !v.trim().is_empty()).ok_or_else(|| anyhow!("give {key}"))
}

fn add(input: &Map<String, Value>, _: &Asked) -> Result<Request> {
    let node = input.get("node").and_then(Value::as_str);
    Ok(Request::post("/v1/add", json!({ "app": text(input, "app")?, "scope": text(input, "scope")?, "added": true, "node": node })))
}

fn remove(input: &Map<String, Value>, _: &Asked) -> Result<Request> {
    Ok(Request::post("/v1/add", json!({ "app": text(input, "app")?, "scope": text(input, "scope")?, "added": false })))
}

fn said(done: &str, input: &Map<String, Value>) -> Result<Value> {
    Ok(json!(format!("{done} {}.", input.get("app").and_then(Value::as_str).unwrap_or("it"))))
}
