//! Groups' commands besides its tools: making and joining groups.

use anyhow::Result;
use serde_json::json;

use ::api::cli::{Command, Given};

/// Making and joining groups, which aren't tools yet: Settings › Groups does them.
pub fn commands() -> Command {
    Command {
        help: vec![
            ("group create <name>".into(), "a group, with you as its admin".into()),
            ("group invite <group> <user> [--admin]".into(), "invite someone to a group, as a member or an admin".into()),
            ("group accept <group>".into(), "join a group you've been invited to".into()),
            ("group remove <group> <user>".into(), "take someone out of a group, or give your own name to leave it".into()),
        ],
        run: cli,
    }
}

fn cli(g: &Given) -> Result<bool> {
    use ::custom_build::config::CLI;
    match g.words().as_slice() {
        ["group", "create", name @ ..] if !name.is_empty() => {
            let r = g.api.post("/api/group", json!({ "name": name.join(" ") }))?;
            println!("Created {} ({}).", name.join(" "), r["group"].as_str().unwrap_or(""));
        }
        ["group", "invite", group, user] => {
            g.api.post("/api/invite", json!({ "group": group, "user": user, "role": if g.has("admin") { "admin" } else { "member" } }))?;
            println!("Invited {user}; they join with {CLI} group accept {group}.");
        }
        ["group", "accept", group] => {
            g.api.post("/api/accept", json!({ "group": group }))?;
            println!("You're in {group}.");
        }
        ["group", "remove", group, user] => {
            g.api.post("/api/member", json!({ "group": group, "user": user, "role": null }))?;
            println!("{user} is no longer in {group}.");
        }
        _ => return Ok(false),
    }
    Ok(true)
}
