//! Who's in the network and this machine, on the command line: status, signing in and out, making or joining a
//! network, and its settings. A command of this program, not the library's: signing in makes this computer one of its
//! user's devices (`client_device`).

use anyhow::{Context, Result, bail};
use one::api::Client;
use one::cli::{Command, Given};
use one::config::{CLI, NAME, YOURS};
use one::rank::Priority;
use one::state::State;
use serde_json::{Value, json};

use one::api;

use crate::{agent, client_device, request};

/// Who's in the network and this device: status, signing in and out, making or joining a network, settings.
pub fn commands() -> Command {
    Command {
        help: vec![
            ("status".into(), format!("this device, and the users, groups and devices in {YOURS}")),
            ("sign-in [--address <url>]".into(), format!("sign this machine in, as you: approve its code by signing in on any of your devices; {CLI} and {CLI} mcp here then act as you. Where {NAME} doesn't run, this computer becomes one of your devices, once you approve its next code too, and asks your devices for everything")),
            ("sign-in --recovery-key <file> [--address <url>]".into(), format!("when no device of yours is left to approve this one: sign in, and this computer lets itself in with {YOURS}'s recovery key, from a file or - for stdin")),
            ("sign-in --invite <link>".into(), "join with the invite link someone sent you, on your first computer: you make your passkey in a browser, and they approve this computer's code".into()),
            ("sign-out".into(), "sign this machine out, and take it out of your devices if it's one".into()),
            ("create <name> --as <your name> [--user <id>]".into(), "create a network, with you as its first admin (--user: who you were in one it replaces, so your sign-in is still yours)".into()),
            ("pair [--user <name>]".into(), "a pairing code for another of your machines (or, for admins, someone else's)".into()),
            ("join <code>".into(), format!("join {YOURS} with a pairing code")),
            ("rename <name>".into(), "rename this device".into()),
            ("role <user> admin|member".into(), "make someone an admin or a member (admins)".into()),
            ("settings [<name> <value>|--default]".into(), "the network's settings (admins set them)".into()),
        ],
        run: |g| {
            let takes = match g.words().as_slice() {
                ["sign-in"] => &["address", "invite", "recovery-key"][..],
                ["status"] => &[],
                ["create", ..] => &["as", "user"],
                ["pair"] => &["user"],
                ["settings", _, ..] => &["default"],
                ["sign-out" | "join" | "settings" | "role" | "rename", ..] => &[],
                _ => return Ok(false),
            };
            g.takes_only(takes)?;
            match g.words().as_slice() {
                ["sign-in"] => sign_in(g)?,
                ["sign-out"] => {
                    // A device takes itself out of its user's devices first, while its sign-in still says who it is; git
                    // here then no longer reaches its repos, which a node here still serves.
                    let address = crate::sign_in::kept().map(|k| k.address).filter(|_| !client_device::node_here());
                    let device = client_device::sign_out();
                    if let Some(address) = address {
                        one::git_wire::unwire(&address);
                    }
                    match crate::sign_in::sign_out()? || device {
                        true if device => println!("Signed out: this computer is no longer one of your devices, and {CLI} here no longer acts as you."),
                        true => println!("Signed out."),
                        false => println!("This machine isn't signed in."),
                    }
                }
                ["status"] if g.json => println!("{}", serde_json::to_string(&g.node.state()?)?),
                ["status"] => status(g.node, &g.node.state()?),
                ["create", name @ ..] if !name.is_empty() => {
                    let user_name = g.flag("as").context("say what to call you with --as <your name>")?;
                    let r = g.node.post("/v1/create", json!({ "name": name.join(" "), "user_name": user_name, "user": g.flag("user") }))?;
                    println!("Created {}.\n\nIts recovery key, shown only this once; keep it in your admins' password manager:\n\n  {}\n\nAdd your other machines with: {CLI} pair", name.join(" "), r["recovery"].as_str().unwrap_or(""));
                }
                ["pair"] => {
                    let r = g.node.post("/v1/pair", json!({ "user": g.flag("user") }))?;
                    println!("On the other machine, run:\n\n  {CLI} join {}\n\nThe pairing code works once, for seven days.", r["code"].as_str().unwrap_or(""));
                }
                ["join", code] => {
                    g.node.post("/v1/join", json!({ "code": code }))?;
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
                        return Err(one::cli::Failed::Typed(format!("no setting called {name}: {}", names.join(", "))).into());
                    }
                    let value = if g.has("default") { Value::Null } else { json!(value.join(" ")) };
                    g.node.post("/v1/settings", json!({ "name": name, "value": value }))?;
                    println!("Set {name}.");
                }
                ["role", _, role] if !matches!(*role, "admin" | "member") => return Err(one::cli::Failed::Typed(format!("a role is admin or member, not {role}")).into()),
                ["role", user, role] => {
                    g.node.post("/v1/role", json!({ "user": user, "role": role }))?;
                    println!("{user} is now {role}.");
                }
                ["rename", name @ ..] if !name.is_empty() => {
                    g.node.post("/v1/name", json!({ "name": name.join(" ") }))?;
                    println!("This device is now {}.", name.join(" "));
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// `sign-in`: this machine signs in at the network's address, as the user who approves its code with their passkey.
fn sign_in(g: &Given) -> Result<()> {
    let node = g.node;
    // Someone new's invite: its link (`<address>/invite#<secret>`), or its secret with --address.
    let invite = g.flag("invite").map(|i| match i.split_once("/invite#") {
        Some((address, secret)) => (Some(address.to_string()), secret.to_string()),
        None => (None, i.trim_start_matches('#').to_string()),
    });
    if invite.is_some() && client_device::node_here() {
        bail!("{NAME} runs on this computer, so it joins as a device, not with an invite: {CLI} join <pairing code>, from {CLI} pair on a device in {YOURS}");
    }
    let address = match g.flag("address").map(str::to_string).or_else(|| invite.as_ref().and_then(|(a, _)| a.clone())) {
        Some(a) => a,
        // With no node here, it's the address it was signed in at before, or the one given.
        None if !client_device::node_here() => crate::sign_in::kept().map(|k| k.address).with_context(|| format!("{NAME} isn't running on this computer, so it signs in as one of your devices: give {YOURS}'s address, {CLI} sign-in --address https://…"))?,
        None => {
            let state: State = serde_json::from_value(request(agent().get(format!("http://{}/v1/state", api::addr())).call())?)?;
            state.address().context(format!("this network has no address to sign in at yet: give one with --address, or an admin sets it with {CLI} cloudflare use"))?.to_string()
        }
    };
    let join = (!client_device::node_here()).then(|| client_device::signing_in_for(&address)).transpose()?;
    let started = crate::sign_in::start(&address, join.as_deref())?;
    let with = if one::config::JOIN_DOMAIN.is_empty() { "your passkey" } else { "your work account" };
    // Someone new joins first, in a browser, which then signs this in as them.
    let link = match &invite {
        Some((_, secret)) => {
            let code = started.url.rsplit_once("c=").map_or("", |(_, c)| c);
            let link = format!("{}/invite?code={code}#{secret}", address.trim_end_matches('/'));
            let how = if one::config::JOIN_DOMAIN.is_empty() { "make your passkey" } else { "sign in with the account it's for" };
            println!("To join, open this link in a browser on any device, phone or computer, and {how}:\n\n  {link}\n");
            link
        }
        None => {
            println!("To sign in this machine, open this link on any of your devices and sign in with {with}:\n\n  {}\n\nor go to {}/sign-in/code and type {}.", started.url, address.trim_end_matches('/'), started.code);
            started.url.clone()
        }
    };
    if crate::sign_in::has_screen() && crate::sign_in::open(&link) {
        println!("It's open in your browser here.");
    }
    crate::sign_in::finish(started)?;
    // A throwaway network's repos are reached by `one-dev throwaway git`, which leaves this machine's git as it is.
    if matches!(one::mode::here(), one::mode::Mode::Own)
        && let Err(err) = one::git_wire::wire(&address)
    {
        eprintln!("{err:#}");
    }
    if client_device::node_here() {
        let me = node.state()?.user.map_or("its user".into(), |u| u.name);
        println!("Signed in. {NAME} runs on this computer, so {CLI} and {CLI} mcp here act as {me}, whose device it is, whoever signs in.");
        return Ok(());
    }
    match g.flag("recovery-key") {
        Some(file) => client_device::join_by_recovery(&address, file)?,
        None => client_device::join(&address, invite.as_ref().map(|(_, secret)| secret.as_str()))?,
    }
    let me = node.state()?.user.map_or("you".into(), |u| u.name);
    println!("\nSigned in as {me}: {} is one of your devices now. {CLI} and {CLI} mcp here act as you, on your devices, until {CLI} sign-out.", crate::sign_in::hostname());
    Ok(())
}

fn status(node: &Client, state: &State) {
    let device = &state.device;
    match client_device::said(state) {
        Some(said) => println!("{said}"),
        None => println!("{} ({}){}", device.name, &device.key[..8.min(device.key.len())], if device.ready { "" } else { ", not ready yet" }),
    }
    let Some(i) = &state.network else {
        println!("\nNot in {YOURS} yet: {CLI} join <code> joins it with a pairing code, or {CLI} create <name> --as <your name> starts a new one.");
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
        let dot = if s.now.online { "●" } else { "○" };
        // As a device, the node that answered is "here" to itself, not to this computer.
        let here = if s.now.here && client_device::me().is_none() { " (this one)" } else { "" };
        let serves = if s.serves_me { "" } else { ", doesn't run yours" };
        let build = s.now.build.as_ref().map(|b| format!("  {b}")).unwrap_or_default();
        println!("  {dot} {}{here}  {}'s{serves}{build}", s.name, name(&s.user));
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
        println!("  ● {}  the Cloudflare node, run priority {at}", one::rank::CLOUDFLARE);
    }
    // A device that's only a client (a phone, a computer, a browser) is listed with them; a passkey isn't one.
    let (passkeys, clients): (Vec<&one::state::Client>, Vec<&one::state::Client>) = i.clients.iter().partition(|c| c.passkey.is_some());
    let this = |c: &one::state::Client| if client_device::me().is_some_and(|(_, key)| c.key == key) { " (this one)" } else { "" };
    for c in &clients {
        let glyph = if is_phone(c) { "▯" } else { "▭" };
        println!("  {glyph} {}{}  {}'s", c.name, this(c), name(&c.user));
    }
    if !passkeys.is_empty() {
        println!("\nPasskeys");
        for c in &passkeys {
            println!("  {}{}  {}", c.name, this(c), name(&c.user));
        }
    }
}

/// Whether a client is a phone, as it says, or for one that hasn't yet, as its name does.
fn is_phone(c: &one::state::Client) -> bool {
    let name = c.name.to_lowercase();
    matches!(c.platform.as_deref(), Some("iphone" | "android")) || (c.platform.is_none() && ["phone", "ipad", "tablet", "android", "pixel", "galaxy"].iter().any(|w| name.contains(w)))
}
