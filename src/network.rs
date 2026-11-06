//! Who's in the network and this machine, on the command line: status, signing in and out, making or joining a
//! network, and its settings. A command of this program, not the library's: signing in makes this computer one of its
//! person's devices (`client_device`).

use anyhow::{Context, Result, bail};
use one::api::Client;
use one::cli::{Command, ago};
use one::config::{CLI, NAME, YOURS};
use serde_json::{Map, Value, json};

use one::api;

use crate::{agent, client_device, request, update, Http};

/// Who's in the network and this device: status, signing in and out, making or joining a network, settings.
pub fn commands() -> Command {
    Command {
        help: vec![
            ("status".into(), format!("this device, and the people, groups and devices in {YOURS}")),
            ("status --until <commit> [--device <name>[,<name>…]] [--timeout <10m>]".into(), format!("wait until every device (or those named) runs that commit, or a later build, and its services answer, or a build fails; says what each did, and fails unless all made it. After {CLI} update: {CLI} status --until $(git rev-parse HEAD)")),
            ("sign-in [--address <url>]".into(), format!("sign this machine in, as you: approve its code by signing in on any of your devices; {CLI} and {CLI} mcp here then act as you. Where {NAME} doesn't run, this computer becomes one of your devices, once you approve its next code too, and asks your devices for everything")),
            ("sign-in --invite <link>".into(), "join with the invite link someone sent you, on your first computer: you make your passkey in a browser, and they approve this computer's code".into()),
            ("sign-out".into(), "sign this machine out, and take it out of your devices if it's one".into()),
            ("create <name> --as <your name> [--user <id>]".into(), "create a network, with you as its first admin (--user: who you were in one it replaces, so your sign-in is still yours)".into()),
            ("pair [--user <name>]".into(), "a pairing code for another of your machines (or, for admins, someone else's)".into()),
            ("join <code>".into(), format!("join {YOURS} with a pairing code")),
            ("rename <name>".into(), "rename this device".into()),
            ("role <user> admin|member".into(), "(admins)".into()),
            ("settings [<name> <value>|--default]".into(), "the network's settings (admins set them)".into()),
        ],
        run: |g| {
            let (flags, args) = (g.flags, g.args);
            match g.words().as_slice() {
                ["sign-in"] => sign_in(g.node, &flags)?,
                ["sign-out"] => {
                    // A device takes itself out of its person's devices first, while its sign-in still says who it is; git
                    // here then no longer reaches its repos, which a node here still serves.
                    let address = crate::sign_in::kept().map(|k| k.address).filter(|_| !client_device::node_here());
                    let device = client_device::sign_out();
                    if let Some(address) = address {
                        one::git_wire::unwire(&address);
                    }
                    match crate::sign_in::sign_out()? || device {
                        true if device => println!("Signed out: this computer is no longer one of your devices, and {CLI} here no longer acts as you."),
                        true => println!("Signed out: {CLI} on this machine no longer acts as you."),
                        false => println!("This machine isn't signed in."),
                    }
                }
                ["status"] if flags.contains_key("until") => update::wait(&args)?,
                ["status"] => status(g.node, &g.node.get("/v1/state")?),
                ["create", name @ ..] if !name.is_empty() => {
                    let user_name = flags.get("as").and_then(Value::as_str).context("say what to call you with --as <your name>")?;
                    let r = unsigned_post("/v1/create", json!({ "name": name.join(" "), "user_name": user_name, "user": flags.get("user") }))?;
                    println!("Created {}.\n\nIts recovery key, shown only this once; keep it in your admins' password manager:\n\n  {}\n\nAdd your other machines with: {CLI} pair", name.join(" "), r["recovery"].as_str().unwrap_or(""));
                }
                ["pair"] => {
                    let r = g.node.post("/v1/pair", json!({ "user": flags.get("user") }))?;
                    println!("On the other machine, run:\n\n  {CLI} join {}\n\nThe pairing code works once, for seven days.", r["code"].as_str().unwrap_or(""));
                }
                ["join", code] => {
                    unsigned_post("/v1/join", json!({ "code": code }))?;
                    println!("Joined. Syncing now; {CLI} status shows progress.");
                }
                ["settings"] => {
                    let state = g.node.get("/v1/state")?;
                    for (name, value) in state["network"]["settings"].as_object().context("not in a network yet")? {
                        println!("{name} = {value}");
                    }
                }
                ["settings", name, value @ ..] => {
                    let value = if flags.contains_key("default") { Value::Null } else { json!(value.join(" ")) };
                    g.node.post("/v1/settings", json!({ "name": name, "value": value }))?;
                    println!("Set {name}.");
                }
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

/// `sign-in`: this machine signs in at the network's address, as the person who approves its code with their passkey.
fn sign_in(node: &Client, flags: &Map<String, Value>) -> Result<()> {
    // Someone new's invite: its link (`<address>/invite#<secret>`), or its secret with --address.
    let invite = flags.get("invite").and_then(Value::as_str).map(|i| match i.split_once("/invite#") {
        Some((address, secret)) => (Some(address.to_string()), secret.to_string()),
        None => (None, i.trim_start_matches('#').to_string()),
    });
    if invite.is_some() && client_device::node_here() {
        bail!("{NAME} runs on this computer, so it joins as a device, not with an invite: {CLI} join <pairing code>, from {CLI} pair on a device in {YOURS}");
    }
    let address = match flags.get("address").and_then(Value::as_str).map(str::to_string).or_else(|| invite.as_ref().and_then(|(a, _)| a.clone())) {
        Some(a) => a,
        // With no node here, it's the address it was signed in at before, or the one given.
        None if !client_device::node_here() => crate::sign_in::kept().map(|k| k.address).with_context(|| format!("{NAME} isn't running on this computer, so it signs in as one of your devices: give {YOURS}'s address, {CLI} sign-in --address https://…"))?,
        None => {
            let state = request(agent().get(format!("http://{}/v1/state", api::addr())).call())?;
            state["network"]["settings"]["address"].as_str().context(format!("this network has no address to sign in at yet: give one with --address, or an admin sets it with {CLI} cloudflare use"))?.to_string()
        }
    };
    let join = (!client_device::node_here()).then(|| client_device::signing_in_for(&address)).transpose()?;
    let started = crate::sign_in::start(&address, join.as_deref())?;
    let with = if one::config::SIGN_IN == one::config::SignIn::Google { "Google" } else { "your passkey" };
    // Someone new joins first, in a browser, which then signs this in as them.
    let link = match &invite {
        Some((_, secret)) => {
            let code = started.url.rsplit_once("c=").map_or("", |(_, c)| c);
            let link = format!("{}/invite?code={code}#{secret}", address.trim_end_matches('/'));
            let how = if one::config::SIGN_IN == one::config::SignIn::Google { "sign in with the Google account it's for" } else { "make your passkey" };
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
    if let Err(err) = one::git_wire::wire(&address) {
        eprintln!("{err:#}");
    }
    if client_device::node_here() {
        let state = node.get("/v1/state")?;
        let me = state["user"]["name"].as_str().unwrap_or("you");
        println!("Signed in as {me}. {CLI} and {CLI} mcp on this machine act as you until {CLI} sign-out.");
        return Ok(());
    }
    client_device::join(&address, invite.as_ref().map(|(_, secret)| secret.as_str()))?;
    let state = node.get("/v1/state")?;
    let me = state["user"]["name"].as_str().unwrap_or("you");
    println!("\nSigned in as {me}: {} is one of your devices now. {CLI} and {CLI} mcp here act as you, on your devices, until {CLI} sign-out.", crate::sign_in::hostname());
    Ok(())
}

/// For a node in no network yet, which no one is signed in to: a sign-in this CLI kept for its address was another
/// node's there before, so it isn't sent, and once the node is in a network it's let go.
fn unsigned_post(path: &str, body: Value) -> Result<Value> {
    let r = Client::new(Http { signed_in: false }, false).post(path, body)?;
    let _ = crate::sign_in::sign_out();
    Ok(r)
}

fn status(node: &Client, state: &Value) {
    let device = &state["device"];
    match client_device::said(state) {
        Some(said) => println!("{said}"),
        None => println!("{} ({}){}", device["name"].as_str().unwrap_or(""), &device["key"].as_str().unwrap_or("")[..8], if device["ready"] == true { "" } else { ", not ready yet" }),
    }
    let i = &state["network"];
    if i.is_null() {
        println!("\nNot in {YOURS} yet: {CLI} join <code> joins it with a pairing code, or {CLI} create <name> --as <your name> starts a new one.");
        return;
    }
    let users: std::collections::HashMap<&str, &str> = i["users"].as_array().into_iter().flatten().filter_map(|u| Some((u["id"].as_str()?, u["name"].as_str()?))).collect();
    let name = |id: &str| users.get(id).copied().unwrap_or(id).to_string();
    println!("\n{}  {} changes", i["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("(joining)"), i["steps"]);
    println!("\nUsers");
    for u in i["users"].as_array().into_iter().flatten() {
        let me = if u["me"] == true { " (you)" } else { "" };
        println!("  {}{me}  {}", u["name"].as_str().unwrap_or(""), u["role"].as_str().unwrap_or(""));
    }
    let groups = i["groups"].as_array().cloned().unwrap_or_default();
    if !groups.is_empty() {
        println!("\nGroups");
        for g in &groups {
            let members: Vec<String> = g["members"].as_array().into_iter().flatten().map(|m| format!("{} ({})", name(m["user"].as_str().unwrap_or("")), m["role"].as_str().unwrap_or("member"))).collect();
            println!("  {}  {}", g["name"].as_str().unwrap_or(""), members.join(", "));
        }
    }
    let apps = node.apps().unwrap_or_default();
    let title = |app: &str| apps.iter().find(|a| a["name"] == app).and_then(|a| a["title"].as_str()).unwrap_or(app).to_string();
    let all = i["devices"].as_array().cloned().unwrap_or_default();
    println!("\nDevices");
    for s in &all {
        let dot = if s["online"] == true { "●" } else { "○" };
        // As a device, the node that answered is "here" to itself, not to this computer.
        let here = if s["here"] == true && client_device::me().is_none() { " (this one)" } else { "" };
        let serves = if s["serves_me"] == true { "" } else { ", doesn't run yours" };
        let build = s["build"].as_str().map(|b| format!("  {b}")).unwrap_or_default();
        println!("  {dot} {}{here}  {}'s{serves}{build}", s["name"].as_str().unwrap_or(""), name(s["user"].as_str().unwrap_or("")));
        // What it runs, as its own list says (its person's and the admins' to see), and the services it says run there;
        // the app runtime isn't one.
        let named = |list: &Value, field: &str| list.as_array().into_iter().flatten().filter_map(|d| d[field].as_str().map(str::to_string)).collect::<Vec<_>>();
        let parts = s["parts"].as_array().cloned().unwrap_or_default();
        let off: std::collections::BTreeSet<String> = named(&s["off_services"], "name").into_iter().chain(parts.iter().filter(|p| p["on"] != true).filter_map(|p| p["name"].as_str().map(str::to_string))).collect();
        let on: std::collections::BTreeSet<String> = named(&s["installed_services"], "name")
            .into_iter()
            .chain(parts.iter().filter(|p| p["on"] == true).filter_map(|p| p["name"].as_str().map(str::to_string)))
            .filter(|n| !off.contains(n))
            .collect();
        let list = |names: &std::collections::BTreeSet<String>| {
            let mut titles: Vec<String> = names.iter().filter(|n| n.as_str() != "apps").map(|n| title(n)).collect();
            titles.sort_by_key(|t| t.to_lowercase());
            titles
        };
        let (on, off) = (list(&on), list(&off));
        if !on.is_empty() || !off.is_empty() {
            let off = if off.is_empty() { String::new() } else { format!("{}off: {}", if on.is_empty() { "" } else { "; " }, off.join(", ")) };
            println!("      Services: {}{off}", on.join(", "));
        }
    }
    // Not a device, but a node like the rest: woken when asked, in its place in run priority.
    if let Some(priority) = i["cloudflare_node"].get("priority") {
        let at = priority.as_u64().map_or("last".to_string(), |n| n.to_string());
        println!("  ● Cloudflare node  run priority {at}");
    }
    // A device that's only a client, a phone or a computer, is listed with them; a browser is a sign-in, not a device.
    let clients = i["clients"].as_array().cloned().unwrap_or_default();
    let (browsers, clients): (Vec<Value>, Vec<Value>) = clients.into_iter().partition(|c| c["passkey"] == true || is_browser(c["name"].as_str().unwrap_or("")));
    let this = |c: &Value| if client_device::me().is_some_and(|(_, key)| c["key"] == key) { " (this one)" } else { "" };
    for c in &clients {
        // A phone keeps no log and holds no sessions: say when it last asked this device, not that it's offline.
        let asked = c["asked"].as_u64().map(|t| format!(", asked here {}", ago(t))).unwrap_or_default();
        let glyph = if is_phone(c["name"].as_str().unwrap_or("")) { "▯" } else { "▭" };
        println!("  {glyph} {}{}  {}'s{asked}", c["name"].as_str().unwrap_or(""), this(c), name(c["user"].as_str().unwrap_or("")));
    }
    if !browsers.is_empty() {
        println!("\nBrowsers");
        for c in &browsers {
            println!("  {}{}  {}", c["name"].as_str().unwrap_or(""), this(c), name(c["user"].as_str().unwrap_or("")));
        }
    }
}

/// What a client's name says it is, as our own apps name themselves ("iPhone", "The One App on Andre’s MacBook Air",
/// "Chrome on Mac"): Settings reads them the same way.
fn is_browser(name: &str) -> bool {
    ["Chrome", "Safari", "Firefox", "Edge", "A browser"].iter().any(|b| name.strip_prefix(b).is_some_and(|rest| rest.is_empty() || rest.starts_with(" on ")))
}

fn is_phone(name: &str) -> bool {
    let name = name.to_lowercase();
    ["phone", "ipad", "tablet", "android", "pixel", "galaxy"].iter().any(|w| name.contains(w))
}
