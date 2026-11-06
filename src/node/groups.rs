//! Groups' tools: who's in the network and in each group, and handles.

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use crate::cli::{Command, Given};
use crate::config::{CLI, YOURS};
use crate::core_tools::{Asked, Feature};
use crate::state::State;

pub fn feature() -> Feature {
    let guide = format!("groups_list says who's in {YOURS}, as admin or member, then its groups: the people who share a page, its apps and its data, like a family or a team, and who's invited to one but hasn't joined. Each person and group has a handle, like andre or family, unique among them, which apps call them by (Repos' addresses are <handle>/<repo>); groups_handle changes yours, or a group's you're an admin of.");
    Feature { name: "groups", title: "Groups", description: format!("Who's in {YOURS}, and in each of its groups"), guide, tools: tools(), elsewhere: true, run }
}

fn tools() -> Vec<Value> {
    vec![
        json!({ "name": "list", "reads": true, "description": format!("The people in {YOURS}, you first, each with their handle, as admin or member; then every group with its handle: who's in it and who's invited but hasn't joined yet."), "input": { "type": "object", "properties": {} } }),
        json!({ "name": "handle", "description": "Change your handle, or a group's you're an admin of: what every app calls you or it by, like Repos' addresses <handle>/<repo>, which move with it.", "input": { "type": "object", "required": ["handle"], "properties": { "handle": { "type": "string", "description": "lowercase letters, digits and single hyphens, up to 39, not taken by anyone or any group, like andre-v" }, "group": { "type": "string", "description": "a group's name or id, from groups_list; omit for your own" } } } }),
    ]
}

fn run(tool: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value> {
    match tool {
        "list" => Ok(json!(said(&asked.node.state()?))),
        "handle" => {
            let handle = crate::settings_tools::HANDLE;
            let (path, body) = (handle.asks)(input).map_err(anyhow::Error::msg)?;
            Ok((handle.said)(input, &asked.node.post(path, body)?))
        }
        other => bail!("no tool groups_{other}"),
    }
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
pub fn commands() -> Command {
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

fn cli(g: &Given) -> Result<bool> {
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
