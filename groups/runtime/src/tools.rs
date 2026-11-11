//! What serves Groups' tools (groups/src/tools.rs).
//! `groups_handle` changes who can do what, so it's answered as this device's own request (`settings::answered`).

use anyhow::Result;
use api::Caller;
use api::runtime::prelude::*;
use api::serves;
use custom_build::config::YOURS;
use scope::scopes::{EVERYONE, ScopeDb};
use serde_json::{Map, Value, json};

/// What serves each of Groups' tools (groups/src/tools.rs).
pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![("list", serves!(list)), ("handle", serves!(handle))]
}

async fn list(rt: Rt, me: Caller, _: Map<String, Value>) -> Result<Value> {
    Ok(json!({ "said": rt.read_or(format!("Users in {YOURS}: none.\nNo groups yet.\n"), |r| said(r.db(), &me.user)) }))
}

/// Asked of this machine's runtime by its own programs, which answers it as its own device.
async fn handle(rt: Rt, _: Caller, input: Map<String, Value>) -> Result<Value> {
    rt.answer_as_device("POST", "/api/groups/handle", &serde_json::to_vec(&input)?).await?;
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

/// The users in the network, `me` first, each with their role; then every group in words: its users with their roles,
/// and who's invited.
fn said(db: &ScopeDb, me: &str) -> Result<String> {
    let all: Vec<_> = db.users()?.into_iter().filter(|u| !u.removed).collect();
    let names: std::collections::HashMap<&str, String> = all.iter().map(|u| (u.id.as_str(), handled(&u.name, &u.handle))).collect();
    let mut users: Vec<_> = all.iter().collect();
    users.sort_by_key(|u| u.id != me);
    let everyone: Vec<String> = users.iter().map(|u| {
        let (name, role) = (handled(&u.name, &u.handle), u.role.as_str());
        if u.id == me { format!("{name} (you, {role})") } else { format!("{name} ({role})") }
    }).collect();
    let mut out = format!("Users in {YOURS} (@{EVERYONE}): {}.\n", everyone.join(", "));
    let users = |list: &std::collections::BTreeMap<String, scope::scopes::Role>| -> Vec<String> {
        list.iter().map(|(user, role)| {
            let (name, role) = (names.get(user.as_str()).map_or(user.as_str(), String::as_str), role.as_str());
            match me == user {
                true => format!("{name} (you, {role})"),
                false => format!("{name} ({role})"),
            }
        }).collect()
    };
    let groups = db.groups()?;
    if groups.is_empty() {
        out.push_str("No groups yet.\n");
        return Ok(out);
    }
    out.push_str("Groups:\n");
    for g in &groups {
        out.push_str(&format!("  {}: {}\n", handled(&g.name, &g.handle), users(&g.members).join(", ")));
        let invited = users(&g.invited);
        if !invited.is_empty() {
            out.push_str(&format!("    invited, not joined yet: {}\n", invited.join(", ")));
        }
    }
    Ok(out)
}
