//! Adding an app and removing it, the two tools of the core that are the one way to do either, each its own route
//! (`api::Runs`): the command line's `add` and `remove`, and an app's screens ask the same ones.

use serde_json::json;

use ::api::Feature;

const GUIDE: &str = "add puts an app, and the apps it uses, on a page: yours (scope user), a group you're an admin of (by its name) or everyone's (scope global, admins only). An app with a service runs it on one of your computers it can run on (node); asked when there's a choice. remove takes it off a page: it stops running there, and what it kept stays, for when it is added back. The App Store (apps_list) lists every app and where it's added.";

fn feature(name: &'static str, title: &'static str, description: &str, tool: serde_json::Value) -> Feature {
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
        json!({
                "name": "add", "title": "Add an app",
                "description": "Add an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only). It shows on that place's page. An app with a service runs it on node, one of your computers it can run on (apps_devices lists them); asked when there's a choice.",
                "input": { "type": "object", "required": ["app", "scope"], "properties": {
                    "app": { "type": "string", "description": "an app's name, from apps_list, like gmail" },
                    "scope": place,
                    "node": { "type": "string", "description": "the computer that runs its service: one of yours by name, as apps_devices lists them" },
                } },
                "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": "what was added" } } },
            }),
    )
}

pub fn remove_feature() -> Feature {
    feature(
        "remove",
        "Remove an app",
        "Remove an app from a place",
        json!({
                "name": "remove", "title": "Remove an app",
                "description": "Remove an app from a place: user (you), a group (by its name) or global (everyone). It leaves that page, for everyone on it, and stops running there; what it kept there stays, and adding it back brings it back.",
                "input": { "type": "object", "required": ["app", "scope"], "properties": {
                    "app": { "type": "string", "description": "an app's name, from apps_list, like gmail" },
                    "scope": { "type": "string", "description": "user (you), global (everyone) or a group's name" },
                } },
                "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": "what was removed" } } },
            }),
    )
}
