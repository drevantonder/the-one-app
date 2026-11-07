//! Apps on the command line: listing, deploying, their code, adding, forking and retiring them, and their schedules.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::api::Client;
use crate::cli::{Command, Given, ago};
use crate::config::CLI;
use crate::{build, tools};

pub fn commands() -> Command {
    Command {
        help: vec![
            ("apps".into(), "every app and its tools".into()),
            ("deploy <dir> [--name <app>] [--owner <owner>]".into(), "build the app in <dir> (app.ts or app.js, and its npm packages) with Bun and deploy it, with its source; makes the app if it's new (owner: you, a group or admins). React screens (screens/<name>.jsx) are built first, into dist/<name>.html, with this machine's Bun or Node".into()),
            ("deploy <app> --version <hash>".into(), "deploy an earlier version again (deploys lists them): a rollback".into()),
            ("deploys <app>".into(), "an app's deploys, newest last".into()),
            ("source <app> [path] [--version <hash>]".into(), "an app's code as deployed: its files, or one of them".into()),
            ("add <app> [--scope user|global|<group>] [--node <device>]".into(), "add an app, and the apps it uses, for you, a group you admin or everyone; --node is the device that runs its service, when it has one and you have more than one".into()),
            ("remove <app> [--scope s]".into(), "".into()),
            ("fork <app> <name> [--owner <owner>]".into(), "a new app from <app>'s code, with data of its own".into()),
            ("app delete|restore|erase <app>".into(), "delete an app you own (to Recently Deleted), restore it, or erase it for good".into()),
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
                    let here = g.node.post("/v1/deploy", json!({ "app": app, "bundle": version, "wait": true }))?["here"] == true;
                    println!("{}", deployed_said(g.node, &format!("Deployed {app} ({version}) again."), here));
                }
                ["deploy", dir] => deploy(g, std::path::Path::new(dir))?,
                ["deploys", app] => {
                    let apps = g.node.apps()?;
                    let found = apps.iter().find(|a| a["name"] == *app).with_context(|| format!("no app named {app}"))?;
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
                    let r = g.node.get(&format!("/v1/source/{app}?{}", q.join("&")))?;
                    match (&r["text"], &r["files"]) {
                        (Value::String(text), _) => print!("{text}"),
                        (_, Value::Array(files)) => files.iter().for_each(|f| println!("{:>8}  {}", f["size"], f["path"].as_str().unwrap_or(""))),
                        _ => bail!("that file isn't text"),
                    }
                }
                ["remove", app] => {
                    let scope = g.flag("scope").unwrap_or("user");
                    g.node.post("/v1/add", json!({ "app": app, "scope": scope, "added": false }))?;
                    println!("{app} is no longer added to {scope}.");
                }
                ["fork", app, name] => {
                    g.node.post("/v1/fork", json!({ "app": app, "name": name, "owner": g.flag("owner") }))?;
                    println!("Forked {app} as {name}; deploy your changes with {CLI} deploy <dir> --name {name}.");
                }
                ["add", app] => {
                    let scope = g.flag("scope").unwrap_or("user");
                    g.node.post("/v1/add", json!({ "app": app, "scope": scope, "added": true, "node": g.flag("node") }))?;
                    println!("{app} is added to {scope}.");
                }
                ["app", how @ ("delete" | "restore" | "erase"), app] => {
                    let past = match *how { "delete" => "deleted", "restore" => "restored", _ => "erased" };
                    g.node.post("/v1/retire", json!({ "app": app, "how": past }))?;
                    println!("{app} is {past}{}.", if *how == "delete" { ": it's in the App Store's Recently Deleted until you restore or erase it" } else { "" });
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
                    g.node.post("/v1/schedule", json!({ "app": app, "schedule": name, "scope": scope, "on": *on == "on" }))?;
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
    let here = node.post("/v1/deploy", json!({ "app": name, "bundle": blob, "owner": g.flag("owner"), "wait": true }))?["here"] == true;
    println!("{}", deployed_said(node, &format!("Deployed {name} ({}).{service}", &blob[..12]), here));
    Ok(())
}

/// What a deploy did, after `done`: which devices run it yet, as far as this one knows (`here`, whether it does).
fn deployed_said(node: &Client, done: &str, here: bool) -> String {
    let device = node.state().map(|s| s.device.name).unwrap_or_else(|_| "This device".into());
    match here {
        true => format!("{done} {device} runs it now; your other devices take it within a few seconds."),
        false => format!("{done} Your devices take it within a few seconds: until then, its calls say they don't have its latest version yet."),
    }
}
