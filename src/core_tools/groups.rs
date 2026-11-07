//! Groups' tools: who's in the network and in each group, and handles.

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use super::{Asked, Feature, Request, Tool};
use crate::config::YOURS;
use crate::state::State;

pub fn feature() -> Feature {
    let guide = format!("groups_list says who's in {YOURS}, as admin or member, then its groups: the people who share a page, its apps and its data, like a family or a team, and who's invited to one but hasn't joined. Each person and group has a handle, like andre or family, unique among them, which apps call them by (Repos' addresses are <handle>/<repo>); groups_handle changes yours, or a group's you're an admin of.");
    Feature { name: "groups", title: "Groups", description: format!("Who's in {YOURS}, and in each of its groups"), guide, tools: tools(), #[cfg(feature = "node")] steps: super::no_steps }
}

fn tools() -> Vec<Tool> {
    vec![
        Tool::asks(json!({ "name": "list", "reads": true, "description": format!("The people in {YOURS}, you first, each with their handle, as admin or member; then every group with its handle: who's in it and who's invited but hasn't joined yet."), "input": { "type": "object", "properties": {} } }), |_, _| Ok(Request::get("/v1/state")), list_said),
        Tool::asks(json!({ "name": "handle", "description": "Change your handle, or a group's you're an admin of: what every app calls you or it by, like Repos' addresses <handle>/<repo>, which move with it.", "input": { "type": "object", "required": ["handle"], "properties": { "handle": { "type": "string", "description": "lowercase letters, digits and single hyphens, up to 39, not taken by anyone or any group, like andre-v" }, "group": { "type": "string", "description": "a group's name or id, from groups_list; omit for your own" } } } }), handle, handle_said),
    ]
}

fn list_said(_: &Map<String, Value>, state: Value) -> Result<Value> {
    Ok(json!(said(&serde_json::from_value(state)?)))
}

fn handle(input: &Map<String, Value>, _: &Asked) -> Result<Request> {
    let handle = super::text(input, "handle")?;
    let group = match input.get("group") {
        None | Some(Value::Null) => None,
        Some(Value::String(g)) => Some(g),
        Some(_) => bail!("group is a group's name or id, from groups_list"),
    };
    Ok(Request::post("/v1/handle", json!({ "handle": handle, "group": group })))
}

fn handle_said(input: &Map<String, Value>, _: Value) -> Result<Value> {
    let handle = input.get("handle").and_then(Value::as_str).unwrap_or("");
    Ok(json!(match input.get("group").and_then(Value::as_str) {
        Some(group) => format!("{group}'s handle is now {handle}."),
        None => format!("Your handle is now {handle}."),
    }))
}

/// A person or group as groups_list says them: their name, then their handle (`Andre @andre`).
fn handled(name: &str, handle: &str) -> String {
    match handle {
        "" => name.to_string(),
        handle => format!("{name} @{handle}"),
    }
}

/// The people in the network (`/v1/state`'s), you first, each with their role; then every group in words: its people
/// with their roles, and who's invited.
pub fn said(state: &State) -> String {
    let Some(i) = &state.network else { return format!("People in {YOURS}: none.\nNo groups yet.\n") };
    let names: std::collections::HashMap<&str, String> = i.users.iter().map(|u| (u.id.as_str(), handled(&u.name, &u.handle))).collect();
    let mut users: Vec<&crate::state::NetworkUser> = i.users.iter().collect();
    users.sort_by_key(|u| !u.me);
    let everyone: Vec<String> = users.iter().map(|u| {
        let (name, role) = (handled(&u.name, &u.handle), u.role.as_str());
        if u.me { format!("{name} (you, {role})") } else { format!("{name} ({role})") }
    }).collect();
    let mut out = format!("People in {YOURS}: {}.\n", everyone.join(", "));
    let people = |list: &[crate::state::Member]| -> Vec<String> {
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
        out.push_str(&format!("  {}: {}\n", handled(&g.name, &g.handle), people(&g.members).join(", ")));
        let invited = people(&g.invited);
        if !invited.is_empty() {
            out.push_str(&format!("    invited, not joined yet: {}\n", invited.join(", ")));
        }
    }
    out
}

/// Making and joining groups, which aren't tools yet: Settings › Groups does them.
#[cfg(feature = "node")]
pub fn commands() -> crate::cli::Command {
    use crate::cli::Command;
    Command {
        help: vec![
            ("group create <name>".into(), "a group, with you as its admin".into()),
            ("group invite <group> <user> [--admin]".into(), "".into()),
            ("group accept <group>".into(), "".into()),
            ("group remove <group> <user>".into(), "or leave it, giving yourself".into()),
        ],
        run: cli,
    }
}

#[cfg(feature = "node")]
fn cli(g: &crate::cli::Given) -> Result<bool> {
    use crate::config::CLI;
    match g.words().as_slice() {
        ["group", "create", name @ ..] if !name.is_empty() => {
            let r = g.node.post("/v1/group", json!({ "name": name.join(" ") }))?;
            println!("Created {} ({}).", name.join(" "), r["group"].as_str().unwrap_or(""));
        }
        ["group", "invite", group, user] => {
            g.node.post("/v1/invite", json!({ "group": group, "user": user, "role": if g.has("admin") { "admin" } else { "member" } }))?;
            println!("Invited {user}; they join with {CLI} group accept {group}.");
        }
        ["group", "accept", group] => {
            g.node.post("/v1/accept", json!({ "group": group }))?;
            println!("You're in {group}.");
        }
        ["group", "remove", group, user] => {
            g.node.post("/v1/member", json!({ "group": group, "user": user, "role": null }))?;
            println!("{user} is no longer in {group}.");
        }
        _ => return Ok(false),
    }
    Ok(true)
}
