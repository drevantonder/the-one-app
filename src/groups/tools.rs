//! Groups' tools, each its own route (`api::Runs`). `handle` changes who can do what, so the caller's
//! client answers it (`settings::answered`), and its route is only this machine's own programs', whose node answers it
//! as its own device.

use serde_json::json;

use ::custom_build::config::YOURS;
use ::api::Feature;

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

/// What serves each tool, in this runtime.
#[cfg(feature = "node")]
pub fn runs() -> ::api::Runs {
    use crate::node::runtime::node_serves;
    ::api::Runs {
        tools: vec![
            ("list", node_serves!(serve::list)),
            ("handle", node_serves!(serve::handle)),
        ],
        ..::api::Runs::named("groups")
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value, json};

    use ::custom_build::config::YOURS;
    use crate::node::Node;
    use ::api::Caller;
    use ::scope::state::State;

    pub async fn list(node: Node, me: Caller, _: Map<String, Value>) -> Result<Value> {
        Ok(json!({ "said": said(&node.state_for(&me)?) }))
    }

    /// Asked of this machine's node by its own programs, which answers it as its own device (`settings`).
    pub async fn handle(node: Node, _: Caller, input: Map<String, Value>) -> Result<Value> {
        node.answer_as_mine("POST", "/api/groups/handle", &serde_json::to_vec(&input)?).await.map_err(|e| e.0)?.0;
        let handle = input.get("handle").and_then(Value::as_str).unwrap_or("");
        Ok(json!({ "said": match input.get("group").and_then(Value::as_str) {
            Some(group) => format!("{group}'s handle is now {handle}."),
            None => format!("Your handle is now {handle}."),
        } }))
    }
    /// A user or group as groups_list says them: their name, then their handle (`Andre @andre`).
    fn handled(name: &str, handle: &str) -> String {
        match handle {
            "" => name.to_string(),
            handle => format!("{name} @{handle}"),
        }
    }

    /// The users in the network (`/api/state`'s), you first, each with their role; then every group in words: its users
    /// with their roles, and who's invited.
    fn said(state: &State) -> String {
        let Some(i) = &state.network else { return format!("Users in {YOURS}: none.\nNo groups yet.\n") };
        let names: std::collections::HashMap<&str, String> = i.users.iter().map(|u| (u.id.as_str(), handled(&u.name, &u.handle))).collect();
        let mut users: Vec<&::scope::state::NetworkUser> = i.users.iter().collect();
        users.sort_by_key(|u| !u.me);
        let everyone: Vec<String> = users.iter().map(|u| {
            let (name, role) = (handled(&u.name, &u.handle), u.role.as_str());
            if u.me { format!("{name} (you, {role})") } else { format!("{name} ({role})") }
        }).collect();
        let mut out = format!("Users in {YOURS} (@{}): {}.\n", ::scope::scopes::EVERYONE, everyone.join(", "));
        let users = |list: &[::scope::state::Member]| -> Vec<String> {
            list.iter().map(|m| {
                let (name, role) = (names.get(m.user.as_str()).map_or(m.user.as_str(), String::as_str), m.role.as_str());
                match i.user == m.user {
                    true => format!("{name} (you, {role})"),
                    false => format!("{name} ({role})"),
                }
            }).collect()
        };
        if i.groups.is_empty() {
            out.push_str("No groups yet.\n");
            return out;
        }
        out.push_str("Groups:\n");
        for g in &i.groups {
            out.push_str(&format!("  {}: {}\n", handled(&g.name, &g.handle), users(&g.members).join(", ")));
            let invited = users(&g.invited);
            if !invited.is_empty() {
                out.push_str(&format!("    invited, not joined yet: {}\n", invited.join(", ")));
            }
        }
        out
    }

}
