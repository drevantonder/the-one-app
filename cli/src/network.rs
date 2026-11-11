//! Who's in the network and this machine, on the command line: status, making or joining a network, and its settings.

use anyhow::Context;
use api::Client;
use api::cli::Command;
use ::custom_build::config::{CLI, YOURS};
use ::scope::rank::Priority;
use ::scope::state::State;
use serde_json::{Value, json};

/// Who's in the network and this device: status, making or joining a network, settings.
pub fn commands() -> Command {
    Command {
        help: vec![
            ("status".into(), format!("this device, and the users, groups and devices in {YOURS}")),
            ("create <name> --as <your name> [--user <id>]".into(), "create a network, with you as its first admin (--user: who you were in one it replaces, so your sign-in is still yours)".into()),
            ("pair [--user <name>]".into(), "a pairing code for another of your machines (or, for admins, someone else's)".into()),
            ("join <code>".into(), format!("join {YOURS} with a pairing code")),
            ("rename <name>".into(), "rename this device".into()),
            ("role <user> admin|member".into(), "make someone an admin or a member (admins)".into()),
            ("settings [<name> <value>|--default]".into(), "the network's settings (admins set them)".into()),
        ],
        run: |g| {
            let takes = match g.words().as_slice() {
                ["status"] => &[][..],
                ["create", ..] => &["as", "user"],
                ["pair"] => &["user"],
                ["settings", _, ..] => &["default"],
                ["join" | "settings" | "role" | "rename", ..] => &[],
                _ => return Ok(false),
            };
            g.takes_only(takes)?;
            match g.words().as_slice() {
                ["status"] if g.json => println!("{}", serde_json::to_string(&g.node.state()?)?),
                ["status"] => status(g.node, &g.node.state()?),
                ["create", name @ ..] if !name.is_empty() => {
                    let user_name = g.flag("as").context("say what to call you with --as <your name>")?;
                    let r = g.node.post("/api/create", json!({ "name": name.join(" "), "user_name": user_name, "user": g.flag("user") }))?;
                    println!("Created {}.\n\nIts recovery key, shown only this once; keep it in your admins' password manager:\n\n  {}\n\nAdd your other machines with: {CLI} pair", name.join(" "), r["recovery"].as_str().unwrap_or(""));
                }
                ["pair"] => {
                    let r = g.node.post("/api/pair", json!({ "user": g.flag("user") }))?;
                    println!("On the other machine, run:\n\n  {CLI} join {}\n\nThe pairing code works once, for seven days.", r["code"].as_str().unwrap_or(""));
                }
                ["join", code] => {
                    g.node.post("/api/join", json!({ "code": code }))?;
                    println!("Joined. Syncing now; {CLI} status shows progress.");
                }
                ["settings"] => {
                    for (name, value) in &g.node.state()?.network.context("not in a network yet")?.settings {
                        println!("{name} = {value:?}");
                    }
                }
                ["settings", name, value @ ..] => {
                    // A lab's setting is named when it's first set; any other name must be one of the network's.
                    let names: Vec<String> = g.node.state()?.network.context("not in a network yet")?.settings.into_keys().collect();
                    if !name.starts_with("lab.") && !names.iter().any(|n| n == name) {
                        return Err(api::cli::Failed::Typed(format!("no setting called {name}: {}", names.join(", "))).into());
                    }
                    let value = if g.has("default") { Value::Null } else { json!(value.join(" ")) };
                    g.node.post("/api/settings", json!({ "name": name, "value": value }))?;
                    println!("Set {name}.");
                }
                ["role", _, role] if !matches!(*role, "admin" | "member") => return Err(api::cli::Failed::Typed(format!("a role is admin or member, not {role}")).into()),
                ["role", user, role] => {
                    g.node.post("/api/role", json!({ "user": user, "role": role }))?;
                    println!("{user} is now {role}.");
                }
                ["rename", name @ ..] if !name.is_empty() => {
                    g.node.post("/api/name", json!({ "name": name.join(" ") }))?;
                    println!("This device is now {}.", name.join(" "));
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

fn status(node: &Client, state: &State) {
    let device = &state.device;
    println!("{} ({}){}", device.name, &device.key[..8.min(device.key.len())], if device.ready { "" } else { ", not ready yet" });
    let Some(i) = &state.network else {
        println!("\n{}.", one::node::not_joined(&device.name));
        return;
    };
    let users: std::collections::HashMap<&str, &str> = i.users.iter().map(|u| (u.id.as_str(), u.name.as_str())).collect();
    let name = |id: &str| users.get(id).copied().unwrap_or(id).to_string();
    println!("\n{}  {} changes", if i.name.is_empty() { "(joining)" } else { &i.name }, i.steps);
    println!("\nUsers");
    for u in &i.users {
        let me = if u.me { " (you)" } else { "" };
        println!("  {}{me}  {}", u.name, u.role.as_str());
    }
    if !i.groups.is_empty() {
        println!("\nGroups");
        for g in &i.groups {
            let members: Vec<String> = g.members.iter().map(|m| format!("{} ({})", name(&m.user), m.role.as_str())).collect();
            println!("  {}  {}", g.name, members.join(", "));
        }
    }
    let apps = node.apps().unwrap_or_default();
    let title = |app: &str| apps.iter().find(|a| a["name"] == app).and_then(|a| a["title"].as_str()).unwrap_or(app).to_string();
    println!("\nDevices");
    for s in &i.devices {
        let presence = Some(api::cli::presence(s.now.online, s.now.seen)).filter(|p| !p.is_empty()).map(|p| format!("  {p}")).unwrap_or_default();
        let here = if s.now.here { " (this one)" } else { "" };
        let serves = if s.serves_me { "" } else { ", doesn't run yours" };
        let build = s.now.build.as_ref().map(|b| format!("  {b}")).unwrap_or_default();
        println!("  {}{here}  {}'s{serves}{build}{presence}", s.name, name(&s.user));
        if let (Some(runs), Some(latest)) = (&s.now.build, &s.now.latest) && runs.get(..12) != latest.get(..12) {
            println!("      behind: runs {runs}, latest is {latest}{}", s.now.updating.as_ref().map_or(String::new(), |why| format!(" ({why})")));
        } else if let Some(why) = &s.now.updating {
            println!("      {why}");
        }
        // What it runs, as its own list says (its user's and the admins' to see), and the services it says run there;
        // the app runtime isn't one.
        let parts = s.now.parts.as_deref().unwrap_or_default();
        let off: std::collections::BTreeSet<&str> = s.off_services.iter().map(|o| o.name.as_str()).chain(parts.iter().filter(|p| !p.on).map(|p| p.name.as_str())).collect();
        let on: std::collections::BTreeSet<&str> = s.now.installed_services.iter().map(|d| d.name.as_str()).chain(parts.iter().filter(|p| p.on).map(|p| p.name.as_str())).filter(|n| !off.contains(n)).collect();
        let list = |names: &std::collections::BTreeSet<&str>| {
            let mut titles: Vec<String> = names.iter().filter(|n| **n != "apps").map(|n| title(n)).collect();
            titles.sort_by_key(|t| t.to_lowercase());
            titles
        };
        let (on, off) = (list(&on), list(&off));
        if !on.is_empty() || !off.is_empty() {
            let off = if off.is_empty() { String::new() } else { format!("{}off: {}", if on.is_empty() { "" } else { "; " }, off.join(", ")) };
            println!("      Services: {}{off}", on.join(", "));
        }
    }
    // Not a device, but a node like the rest: woken when asked, in its place in run priority, and named as any is.
    if let Some(cloudflare) = &i.cloudflare_node {
        let at = match cloudflare.priority {
            Priority::Number(n) => n.to_string(),
            Priority::Last => "last".to_string(),
        };
        println!("  {}  the Cloudflare node, run priority {at}", ::scope::rank::CLOUDFLARE);
    }
    // A device that's only a client (a phone, a computer, a browser) is listed with them; a passkey isn't one, and a
    // device with an app runtime of its own is listed as that runtime, above.
    let (passkeys, clients): (Vec<&::scope::state::Client>, Vec<&::scope::state::Client>) = i.clients.iter().partition(|c| c.passkey.is_some());
    for c in clients.iter().filter(|c| !i.devices.iter().any(|d| d.device.as_ref() == Some(&c.key))) {
        let glyph = if is_phone(c) { "▯" } else { "▭" };
        println!("  {glyph} {}  {}'s", c.name, name(&c.user));
    }
    if !passkeys.is_empty() {
        println!("\nPasskeys");
        for c in &passkeys {
            println!("  {}  {}", c.name, name(&c.user));
        }
    }
}

/// Whether a client is a phone, as it says, or for one that hasn't yet, as its name does.
fn is_phone(c: &::scope::state::Client) -> bool {
    let name = c.name.to_lowercase();
    matches!(c.platform.as_deref(), Some("iphone" | "android")) || (c.platform.is_none() && ["phone", "ipad", "tablet", "android", "pixel", "galaxy"].iter().any(|w| name.contains(w)))
}
