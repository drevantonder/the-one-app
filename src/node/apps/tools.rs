//! Adding an app and removing it, the two tools of the core that are the one way to do either: the command line's
//! `add` and `remove` ask the same route (`/v1/add`), and an app's screens through its own `ctx.add`.

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use crate::core_tools::{Asked, Feature};
use crate::settings_tools::{ADD, REMOVE, Tool};

const GUIDE: &str = "add puts an app, and the apps it uses, on a page: yours (scope user), a group you're an admin of (by its name) or everyone's (scope global, admins only). An app with a service runs it on one of your nodes it can run on (node), or the Cloudflare node for everyone's or a group's; asked when there's a choice. remove takes it off a page, and what it keeps there goes with it. The App Store (apps_list) lists every app and where it's added.";

fn feature(name: &'static str, title: &'static str, description: &str, tool: Value) -> Feature {
    Feature { name, title, description: description.into(), guide: GUIDE.into(), tools: vec![tool], elsewhere: true, run }
}

pub fn add_feature() -> Feature {
    let place = json!({ "type": "string", "description": "user (you), global (everyone) or a group's name" });
    let input = json!({ "type": "object", "required": ["app", "scope"], "properties": {
        "app": { "type": "string", "description": "an app's name, from apps_list, like gmail" },
        "scope": place,
        "node": { "type": "string", "description": "the node that runs its service: one of yours by name, or cloudflare, as apps_devices lists them" },
    } });
    feature("add", "Add an app", "Add an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only)", json!({ "name": "add", "description": "Add an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only). It shows on that place's page. An app with a service runs it on node, one of your nodes it can run on (apps_devices lists them), or for everyone's or a group's, the Cloudflare node; asked when there's a choice.", "input": input }))
}

pub fn remove_feature() -> Feature {
    let input = json!({ "type": "object", "required": ["app", "scope"], "properties": {
        "app": { "type": "string", "description": "an app's name, from apps_list, like gmail" },
        "scope": { "type": "string", "description": "user (you), global (everyone) or a group's name" },
    } });
    feature("remove", "Remove an app", "Remove an app from a place", json!({ "name": "remove", "confirm": true, "description": "Remove an app from a place: user (you), a group (by its name) or global (everyone). What it keeps there goes with it.", "input": input }))
}

fn run(tool: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value> {
    let tool: &Tool = match tool {
        "add" => &ADD,
        "remove" => &REMOVE,
        other => bail!("no tool {other}"),
    };
    let (path, body) = (tool.asks)(input).map_err(anyhow::Error::msg)?;
    Ok((tool.said)(input, &asked.node.post(path, body)?))
}
