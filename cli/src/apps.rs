//! Apps on the command line: listing, deploying, their code, adding and forking them, and their schedules.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use ::api::Client;
use ::api::cli::{Command, Given, ago};
use ::custom_build::config::CLI;
use one::{build, tools};

pub fn commands() -> Command {
    Command {
        help: vec![
            ("apps".into(), "every app and its tools".into()),
            ("deploy <dir> [--name <app>] [--owner <owner>]".into(), "build the app in <dir> (app.ts or app.js, and its npm packages) with Bun and deploy it, with its source; makes the app if it's new (owner: you, a group or admins). React screens (screens/<name>.jsx) are built first, into dist/<name>.html, with this machine's Bun or Node".into()),
            ("deploy <app> --version <hash>".into(), "deploy an earlier version again (deploys lists them): a rollback".into()),
            ("deploys <app>".into(), "an app's deploys, newest last".into()),
            ("source <app> [path] [--version <hash>]".into(), "an app's code as deployed: its files, or one of them".into()),
            ("add <app> [--scope user|global|<group>] [--node <device>]".into(), "add an app, and the apps it uses, for you, a group you admin or everyone; --node is the device that runs its service, when it has one and you have more than one".into()),
            ("remove <app> [--scope user|global|<group>]".into(), "remove an app from a place: it stops running there, and what it kept there stays for if you add it back".into()),
            ("fork <app> <name> [--owner <owner>]".into(), "a new app from <app>'s code, with data of its own".into()),
            ("schedules".into(), "every schedule of the apps added for you: when, as whom, and how it last went".into()),
            ("schedule on|off <app> <name> [--scope user|global|<group>]".into(), "switch an app's schedule on (it then runs as you) or off".into()),
        ],
        run: |g| {
            match g.words().as_slice() {
                ["apps"] => {
                    let groups = g.node.state()?.network.map(|n| n.groups).unwrap_or_default();
                    // Where it's added, as the App Store says it: you, a group by its name, everyone.
                    let place = |scope: &str| match scope.split_once(':') {
                        Some(("group", id)) => groups.iter().find(|g| g.id == id).map_or(id, |g| g.name.as_str()).to_string(),
                        Some(("user", _)) => "you".to_string(),
                        _ => "everyone".to_string(),
                    };
                    for app in &g.node.apps()? {
                        let from = app["from"].as_str().map(|f| format!(", a fork of {f}")).unwrap_or_default();
                        let added: Vec<String> = app["added"].as_array().into_iter().flatten().filter_map(Value::as_str).map(place).collect();
                        let added = if added.is_empty() { String::new() } else { format!(", added for {}", added.join(", ")) };
                        println!("{} ({}{from}{added})", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or(""));
                        tools::list(std::slice::from_ref(app)).iter().for_each(|l| println!("{l}"));
                    }
                }
                ["deploy", app] if g.has("version") => {
                    let version = g.flag("version").context("give --version <hash>")?;
                    let deployed = g.node.post("/api/deploy", json!({ "app": app, "bundle": version, "wait": true }))?;
                    deployed_said(g.node, app, &deployed, "")?;
                }
                ["deploy", dir] => deploy(g, std::path::Path::new(dir))?,
                ["deploys", app] => {
                    let apps = g.node.apps()?;
                    let found = apps.iter().find(|a| a["name"] == *app).with_context(|| format!("no app named {app}: {CLI} apps lists them"))?;
                    for d in found["history"].as_array().into_iter().flatten() {
                        let current = if d["bundle"] == found["bundle"] { "  (running)" } else { "" };
                        println!("{}  {}{current}", &d["bundle"].as_str().unwrap_or("")[..12], ago(d["time"].as_u64().unwrap_or(0)));
                    }
                }
                ["source", app, rest @ ..] => {
                    let mut q = Vec::new();
                    if let Some(b) = g.flag("version") {
                        q.push(format!("bundle={b}"));
                    }
                    if let [path] = rest {
                        q.push(format!("path={path}"));
                    }
                    let r = g.node.get(&format!("/api/source/{app}?{}", q.join("&")))?;
                    match (&r["text"], &r["files"]) {
                        (Value::String(text), _) => print!("{text}"),
                        (_, Value::Array(files)) => files.iter().for_each(|f| println!("{:>8}  {}", f["size"], f["path"].as_str().unwrap_or(""))),
                        _ => bail!("that file isn't text"),
                    }
                }
                ["remove", app] => {
                    let scope = g.flag("scope").unwrap_or("user");
                    println!("{}", g.node.post("/api/remove/remove", json!({ "app": app, "scope": scope }))?["said"].as_str().unwrap_or_default());
                }
                ["fork", app, name] => {
                    g.node.post("/api/fork", json!({ "app": app, "name": name, "owner": g.flag("owner") }))?;
                    println!("Forked {app} as {name}; deploy your changes with {CLI} deploy <dir> --name {name}.");
                }
                ["add", app] => {
                    let scope = g.flag("scope").unwrap_or("user");
                    println!("{}", g.node.post("/api/add/add", json!({ "app": app, "scope": scope, "node": g.flag("node") }))?["said"].as_str().unwrap_or_default());
                }
                ["schedules"] => {
                    for app in &g.node.apps()? {
                        for s in app["schedules"].as_array().into_iter().flatten() {
                            let state = match (&s["invalid"], s["removed"] == true, s["on"] == true) {
                                (Value::String(why), ..) => format!("doesn't make sense: {why}"),
                                (_, true, _) => "removed by a deploy".into(),
                                (_, _, true) => format!("{}, as {}", s["when"].as_str().unwrap_or(""), s["runs_as"].as_str().unwrap_or("nobody")),
                                _ => "off".into(),
                            };
                            let last = match &s["last"] {
                                Value::Null => String::new(),
                                l if l["ok"] == true => format!("; ran {}", ago(l["time"].as_u64().unwrap_or(0))),
                                l => format!("; failed {}: {}", ago(l["time"].as_u64().unwrap_or(0)), l["error"].as_str().unwrap_or("")),
                            };
                            println!("{} {}  in {}  {state}{last}", app["name"].as_str().unwrap_or(""), s["name"].as_str().unwrap_or(""), s["scope"].as_str().unwrap_or(""));
                        }
                    }
                }
                ["schedule", on @ ("on" | "off"), app, name] => {
                    let scope = g.flag("scope").unwrap_or("user");
                    g.node.post("/api/schedule", json!({ "app": app, "schedule": name, "scope": scope, "on": *on == "on" }))?;
                    println!("{app}'s schedule {name} is {on} in {scope}.");
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Deploys the app in `dir`: builds it with its source into a bundle, makes the app if the network hasn't one by that
/// name, and deploys it, saying whether this device runs it yet.
fn deploy(g: &Given, dir: &std::path::Path) -> Result<()> {
    let node = g.node;
    anyhow::ensure!(dir.is_dir(), "no folder {} in {}: give the app's folder", dir.display(), std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default());
    let name = match g.flag("name") {
        Some(name) => name.to_string(),
        None => dir.canonicalize()?.file_name().context("which app?")?.to_string_lossy().to_string(),
    };
    let (blob, service) = build::built_with_service(node, dir)?;
    let deployed = node.post("/api/deploy", json!({ "app": name, "bundle": blob, "owner": g.flag("owner"), "wait": true }))?;
    deployed_said(node, &name, &deployed, &service)?;
    Ok(())
}

/// Says what the deploy did. One that doesn't start is deployed, but this command failed.
fn deployed_said(node: &Client, app: &str, deployed: &Value, service: &str) -> Result<()> {
    let said = format!("{}{service}", ::deploy::build::said(node, app, deployed, ::api::Reader::Cli));
    if deployed["failed"].is_string() {
        anyhow::bail!("{said}");
    }
    println!("{said}");
    Ok(())
}
