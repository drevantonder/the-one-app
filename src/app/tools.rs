//! Adding an app and removing it, the two tools of the core that are the one way to do either, each its own route
//! (`core_tools::Runs::Route`): the command line's `add` and `remove`, and an app's screens ask the same ones.

use serde_json::json;

use crate::core_tools::{Feature, Tool, serves};

const GUIDE: &str = "add puts an app, and the apps it uses, on a page: yours (scope user), a group you're an admin of (by its name) or everyone's (scope global, admins only). An app with a service runs it on one of your nodes it can run on (node), or the Cloudflare node for everyone's or a group's; asked when there's a choice. remove takes it off a page, and what it keeps there goes with it. The App Store (apps_list) lists every app and where it's added.";

fn feature(name: &'static str, title: &'static str, description: &str, tool: Tool) -> Feature {
    Feature {
        name,
        title,
        description: description.into(),
        guide: GUIDE.into(),
        tools: vec![tool],
    }
}

pub fn add_feature() -> Feature {
    let place = json!({ "type": "string", "description": "user (you), global (everyone) or a group's name" });
    feature(
        "add",
        "Add an app",
        "Add an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only)",
        Tool::route(
            json!({
                "name": "add", "title": "Add an app",
                "description": "Add an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only). It shows on that place's page. An app with a service runs it on node, one of your nodes it can run on (apps_devices lists them), or for everyone's or a group's, the Cloudflare node; asked when there's a choice.",
                "input": { "type": "object", "required": ["app", "scope"], "properties": {
                    "app": { "type": "string", "description": "an app's name, from apps_list, like gmail" },
                    "scope": place,
                    "node": { "type": "string", "description": "the node that runs its service: one of yours by name, or cloudflare, as apps_devices lists them" },
                } },
                "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": "what was added" } } },
            }),
            serves!(serve::add),
        ),
    )
}

pub fn remove_feature() -> Feature {
    feature(
        "remove",
        "Remove an app",
        "Remove an app from a place",
        Tool::route(
            json!({
                "name": "remove", "title": "Remove an app", "confirm": true,
                "description": "Remove an app from a place: user (you), a group (by its name) or global (everyone). What it keeps there goes with it.",
                "input": { "type": "object", "required": ["app", "scope"], "properties": {
                    "app": { "type": "string", "description": "an app's name, from apps_list, like gmail" },
                    "scope": { "type": "string", "description": "user (you), global (everyone) or a group's name" },
                } },
                "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": "what was removed" } } },
            }),
            serves!(serve::remove),
        ),
    )
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value, json};

    use crate::core_tools::text;
    use crate::node::{Caller, Node};

    /// Adds `app` and the apps it uses in `scope` (user, `group:<id>` or a group's name, or global), its services on
    /// `node` (a device by name, or cloudflare).
    pub async fn add(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let (app, scope) = (text(&input, "app")?, node.scope_for(&me, text(&input, "scope")?)?);
        node.add(&me, app, &scope, true, input.get("node").and_then(Value::as_str)).await?;
        Ok(json!({ "said": format!("Added {app}.") }))
    }

    pub async fn remove(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let (app, scope) = (text(&input, "app")?, node.scope_for(&me, text(&input, "scope")?)?);
        node.add(&me, app, &scope, false, None).await?;
        Ok(json!({ "said": format!("Removed {app}.") }))
    }
}
