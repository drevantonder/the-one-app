//! Groups' tools, each its own route: what it takes and answers. What serves them is the runtime's
//! (groups/runtime/src/tools.rs). `handle` changes who can do what, so the caller's client answers it
//! (`settings::answered`), and its route is only this machine's own programs', whose runtime answers it as its own device.

use serde_json::json;

use ::api::Feature;
use ::custom_build::config::YOURS;

pub fn feature() -> Feature {
    let guide = format!("groups_list says who's in {YOURS}, as admin or member, then its groups: the users who share a page, its apps and its data, like a family or a team, and who's invited to one but hasn't joined. Each user and group has a handle, like andre or family, unique among them, which apps call them by (Repos' addresses are <handle>/<repo>); groups_handle changes yours, or a group's you're an admin of.");
    Feature { name: "groups", title: "Groups", description: format!("Who's in {YOURS}, and in each of its groups"), guide, tools: tools(), }
}

fn tools() -> Vec<serde_json::Value> {
    vec![
        json!({
                "name": "list", "title": "Who's in the network and its groups", "reads": true,
                "description": format!("The users in {YOURS}, you first, each with their handle, as admin or member; then every group with its handle: who's in it and who's invited but hasn't joined yet."),
                "input": { "type": "object", "properties": {} },
                "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": "the users and groups, in words" } } },
            }),
        json!({
                "name": "handle", "title": "Change a handle",
                "description": "Change your handle, or a group's you're an admin of: what every app calls you or it by, like Repos' addresses <handle>/<repo>, which move with it.",
                "input": { "type": "object", "required": ["handle"], "properties": {
                    "handle": { "type": "string", "description": "lowercase letters, digits and single hyphens, up to 39, not taken by anyone or any group, like andre-v" },
                    "group": { "type": "string", "description": "a group's name or id, from groups_list; omit for your own" },
                } },
                "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": "whose handle it is now" } } },
            }),
    ]
}
