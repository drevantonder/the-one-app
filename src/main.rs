
use anyhow::{Context, Result, bail};
use one::config::{CLI, NAME, YOURS};
use one::cli::{Command, Given};
use one::skills::Mode;
use one::api;
use serde_json::{Map, Value, json};

mod client_device;
mod git_remote;
mod throwaway;
mod update;

use one::api::Client;
use one::{build, code, tools};

/// Every built-in command, each feature's, in the order they're tried: the first that takes what was typed runs it.
fn commands() -> Vec<Command> {
    use one::node::{backups, groups, storage, vault};
    vec![network(), groups::commands(), apps(), vault::cli::commands(), one::cloudflare::cli::commands(), storage::cli::commands(), backups::cli::commands(), running(), releases(), agents(), throwaway::command(), generic()]
}

/// Every built-in command, with what it takes: `help all` prints it, and `<command> --help` its lines.
fn every_command() -> String {
    format!("Usage:\n{}\n{NAME} holds one network. For another context, run that context's own app.", one::cli::help(&commands()))
}

fn command_words() -> Vec<String> {
    let mut out: Vec<String> = commands().iter().flat_map(|c| &c.help).filter_map(|(usage, _)| usage.split_whitespace().next()).filter(|w| w.chars().all(|c| c.is_ascii_lowercase() || c == '-')).map(String::from).collect();
    out.dedup();
    out
}

/// What `help` and the bare command say: what an agent needs to start, and where the rest is.
fn help(node: &Client) -> String {
    let apps = node.get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
    let names: Vec<&str> = apps.iter().filter(|a| a["ops"].as_array().is_some_and(|o| !o.is_empty())).filter_map(|a| a["name"].as_str()).collect();
    let yours = match names.is_empty() {
        true => String::new(),
        false => format!("\nYour apps: {}.\n", names.join(", ")),
    };
    format!(
        "{NAME}: your machines, and your team's, as one place for apps and agents.

  {CLI} search <words>        the tools for a task in every app, each with how to call it
  {CLI} <app>                 an app's tools and its guide
  {CLI} <app> <tool> --help     what a tool takes
  {CLI} <app> <tool> [words] [--input value]
                             run one, like {CLI} repos list
  {CLI} run <file>            a script that calls several tools; {CLI} search gives each call
  {CLI} skills                the guides agents load
  {CLI} status                who you are, and your devices
  {CLI} mcp [--code]          the local MCP server, for agents
{yours}
A tool's words fill its main inputs in order, a list repeats its flag, and inputs not given are read as JSON
from stdin. --file <input>=<path> gives a file, --out <path> saves what comes back, --device <name> runs it there,
and --confirm goes ahead with what sends or can't be undone, once the person agreed. --json prints what comes back
as JSON. --trace prints what the call did, and where, after its answer: use it, and {CLI} logs <device>, before ssh.

Exit codes: 0 done, 2 what was typed is wrong, 3 refused (needs --confirm, not allowed, or not signed in), 1 anything
else. Data goes to stdout, messages to stderr.

{CLI} help all lists every command; {CLI} <command> --help says what one takes."
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if one::oned::invoked() {
        if let Err(err) = one::oned::run(&args) {
            eprintln!("{}: {err:#}", one::oned::name());
            std::process::exit(1);
        }
        return;
    }
    // Run by git, for a repo's address.
    if std::env::args_os().next().is_some_and(|me| std::path::Path::new(&me).file_name().is_some_and(|n| *n == *one::git_wire::helper())) {
        let ran = git_remote::run(&args);
        client_device::end();
        match ran {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!("{CLI}: {err:#}");
                std::process::exit(128);
            }
        }
    }
    let ran = run(args);
    client_device::end();
    if let Err(err) = ran {
        // What the node says names its tools (apps_restore); the command line names them as commands.
        eprintln!("{CLI}: {}", tools::in_cli_words(&node(), &format!("{err:#}")));
        std::process::exit(tools::exit_code(&err));
    }
}

fn run(args: Vec<String>) -> Result<()> {
    // A throwaway's command is run again as a program of its network, which reads what follows itself.
    if args.first().map(String::as_str) == Some("throwaway") {
        return throwaway::run(&args[1..]);
    }
    // --json is everyone's, wherever it's typed: what a tool gives back, as JSON.
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let json = args[..end].iter().any(|a| a == "--json");
    // --trace too: a call prints what it did, and where, after its answer.
    let trace = args[..end].iter().any(|a| a == "--trace");
    let args: Vec<String> = args.iter().enumerate().filter(|(i, a)| *i >= end || (*a != "--json" && *a != "--trace")).map(|(_, a)| a.clone()).collect();
    let (positional, flags) = parse(&args);
    if positional.first().map(String::as_str) != Some("mcp") {
        // Printing into `| head` ends quietly, as other commands do, instead of panicking.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    let (agent, node, words) = (one::mcp::agent_from_env(), node(), command_words());
    let given = Given { words: &positional, flags: &flags, args: &args, node: &node, agent: agent.as_deref(), json, trace, commands: &words };
    // --help shows help, whatever the command: never runs it. An app's tool's is the app's own.
    if flags.contains_key("help") || positional.iter().any(|p| p == "-h") {
        match command_help(&node, &positional) {
            Some(text) => print!("{text}"),
            None => _ = (generic().run)(&given)?,
        }
        return Ok(());
    }
    for command in commands() {
        if (command.run)(&given)? {
            return Ok(());
        }
    }
    unreachable!("any word is an app's name, if not a command's")
}

/// Who's in the network and this device: status, signing in and out, making or joining a network, settings.
fn network() -> Command {
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
                    let address = one::sign_in::kept().map(|k| k.address).filter(|_| !client_device::node_here());
                    let device = client_device::sign_out();
                    if let Some(address) = address {
                        one::git_wire::unwire(&address);
                    }
                    match one::sign_in::sign_out()? || device {
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

/// Apps: listing, deploying, their code, adding them and forking them.
fn apps() -> Command {
    Command {
        help: vec![
            ("apps".into(), "every app and its tools".into()),
            ("deploy <dir> [--name <app>] [--owner <owner>]".into(), "build the app in <dir> (app.ts or app.js, and its npm packages) with Bun and deploy it, with its source; makes the app if it's new (owner: you, a group or admins). React screens (screens/<name>.jsx) are built first, into dist/<name>.html, with this machine's Bun or Node".into()),
            ("deploy <app> --version <hash>".into(), "deploy an earlier version again (deploys lists them): a rollback".into()),
            ("deploys <app>".into(), "an app's deploys, newest last".into()),
            ("source <app> [path] [--version <hash>]".into(), "an app's code as deployed: its files, or one of them".into()),
            ("install <app> [--scope user|global|<group>]".into(), "install an app, and the apps it uses, for you, a group you admin or everyone".into()),
            ("uninstall <app> [--scope s]".into(), "".into()),
            ("fork <app> <name> [--owner <owner>]".into(), "a new app from <app>'s code, with data of its own".into()),
        ],
        run: |g| {
            let flags = g.flags;
            match g.words().as_slice() {
                ["apps"] => {
                    let groups = g.node.get("/v1/state")?["network"]["groups"].clone();
                    // Where it's added, as the App Store says it: you, a group by its name, everyone.
                    let place = |scope: &str| match scope.split_once(':') {
                        Some(("group", id)) => groups.as_array().into_iter().flatten().find(|g| g["id"] == id).and_then(|g| g["name"].as_str()).unwrap_or(id).to_string(),
                        Some(("user", _)) => "you".to_string(),
                        _ => "everyone".to_string(),
                    };
                    for app in g.node.get("/v1/apps")?.as_array().into_iter().flatten() {
                        let from = app["from"].as_str().map(|f| format!(", a fork of {f}")).unwrap_or_default();
                        let installed: Vec<String> = app["installed"].as_array().into_iter().flatten().filter_map(Value::as_str).map(place).collect();
                        let installed = if installed.is_empty() { String::new() } else { format!(", added for {}", installed.join(", ")) };
                        println!("{} ({}{from}{installed})", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or(""));
                        tools::list(std::slice::from_ref(app)).iter().for_each(|l| println!("{l}"));
                    }
                }
                ["deploy", app] if flags.contains_key("version") => {
                    let version = flags["version"].as_str().context("give --version <hash>")?;
                    let here = g.node.post("/v1/deploy", json!({ "app": app, "bundle": version, "wait": true }))?["here"] == true;
                    println!("{}", deployed_said(g.node, &format!("Deployed {app} ({version}) again."), here));
                }
                ["deploy", dir] => deploy(g.node, std::path::Path::new(dir), &flags)?,
                ["deploys", app] => {
                    let apps = g.node.get("/v1/apps")?;
                    let found = apps.as_array().into_iter().flatten().find(|a| a["name"] == *app).with_context(|| format!("no app named {app}"))?;
                    for d in found["history"].as_array().into_iter().flatten() {
                        let current = if d["bundle"] == found["bundle"] { "  (running)" } else { "" };
                        println!("{}  {}{current}", &d["bundle"].as_str().unwrap_or("")[..12], ago(d["time"].as_u64().unwrap_or(0)));
                    }
                }
                ["source", app, rest @ ..] => {
                    let mut q = Vec::new();
                    if let Some(b) = flags.get("version").and_then(Value::as_str) {
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
                ["uninstall", app] => {
                    let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
                    g.node.post("/v1/install", json!({ "app": app, "scope": scope, "installed": false }))?;
                    println!("{app} is no longer installed in {scope}.");
                }
                ["fork", app, name] => {
                    g.node.post("/v1/fork", json!({ "app": app, "name": name, "owner": flags.get("owner") }))?;
                    println!("Forked {app} as {name}; deploy your changes with {CLI} deploy <dir> --name {name}.");
                }
                ["install", app] => {
                    let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
                    g.node.post("/v1/install", json!({ "app": app, "scope": scope, "installed": true }))?;
                    println!("{app} is installed in {scope}.");
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}


/// What runs where: apps deleted and restored, schedules, services and logs.
fn running() -> Command {
    Command {
        help: vec![
            ("app delete|restore|erase <app>".into(), "delete an app you own (to Recently Deleted), restore it, or erase it for good".into()),
            ("schedules".into(), "every schedule of the apps installed for you: when, as whom, and how it last went".into()),
            ("services".into(), "what each of your devices can run, each on or off, and what it's doing".into()),
            ("services on|off <name> [--device <name>]".into(), "switch a service on or off on a device of yours (here by default), which its oned starts or stops, fetching its program as it's published (a Mac app carries its own); apps is its app service. An admin's --device cloudflare switches a service built for Cloudflare on the Cloudflare node (Models: Workers AI)".into()),
            ("services sandbox | sudo sh".into(), "let this machine run services in their sandbox, each as its own user (Linux; the installer does it)".into()),
            ("services try apps/<name> [--device <name>]".into(), "put your checkout's build of an app's service on a device of yours (here by default, else over ssh by its name) and switch it on, until a newer one is published".into()),
            ("logs <device> [--service <name>] [--since <10m>] [--grep <text>] [--lines <n>]".into(), "a device's node log, or one of its services', read over the network, newest last, capped (the last 200 lines, at most 1000 or 48 KiB); only its person and the network's admins read them. --since takes s, m, h or d".into()),
            ("schedule on|off <app> <name> [--scope user|global|<group>]".into(), "switch an app's schedule on (it then runs as you) or off".into()),
        ],
        run: |g| {
            let flags = g.flags;
            match g.words().as_slice() {
                ["app", how @ ("delete" | "restore" | "erase"), app] => {
                    let past = match *how { "delete" => "deleted", "restore" => "restored", _ => "erased" };
                    g.node.post("/v1/retire", json!({ "app": app, "how": past }))?;
                    println!("{app} is {past}{}.", if *how == "delete" { ": it's in the App Store's Recently Deleted until you restore or erase it" } else { "" });
                }
                ["services"] => {
                    let state = g.node.get("/v1/state")?;
                    let all: Vec<Value> = state["network"]["devices"].as_array().cloned().unwrap_or_default();
                    for s in &all {
                        let installed = s["installed_services"].as_array().cloned().unwrap_or_default();
                        let off = s["off_services"].as_array().cloned().unwrap_or_default();
                        if let Some(parts) = s["parts"].as_array() {
                            println!("{}{}", s["name"].as_str().unwrap_or(""), if s["online"] == true { "" } else { " (offline)" });
                            for p in parts {
                                let name = p["name"].as_str().unwrap_or("");
                                let version = installed.iter().find(|d| d["name"] == name).and_then(|d| d["version"].as_str()).map(|v| format!(" {}", &v[..v.len().min(12)])).unwrap_or_default();
                                let doing = p["doing"].as_str().map(|d| format!(", {}", d.to_lowercase())).unwrap_or_default();
                                let problem = p["problem"].as_str().map(|w| format!(": {w}")).unwrap_or_default();
                                let held = match (p["sandbox"].as_str(), p["person"] == true) {
                                    (Some(why), _) => format!(", without its sandbox: {why}"),
                                    (None, true) => ", as you".into(),
                                    _ => String::new(),
                                };
                                println!("  {name}{version}  {}{doing}{held}{problem}", if p["on"] == true { "on" } else { "off" });
                            }
                            continue;
                        }
                        if installed.is_empty() && off.is_empty() {
                            continue;
                        }
                        println!("{}{}", s["name"].as_str().unwrap_or(""), if s["online"] == true { "" } else { " (offline)" });
                        for d in &installed {
                            let by = match (&d["installed_by"], d["built_in"] == true) {
                                (_, true) => format!("built into {NAME}"),
                                (Value::String(by), _) => format!("installed by {by}"),
                                _ => String::new(),
                            };
                            let version = d["version"].as_str().map(|v| format!(" {}", &v[..v.len().min(12)])).unwrap_or_default();
                            println!("  {}{version}  {} ({by})", d["name"].as_str().unwrap_or(""), d["title"].as_str().unwrap_or(""));
                        }
                        for d in &off {
                            println!("  {}  switched off {}", d["name"].as_str().unwrap_or(""), ago(d["time"].as_u64().unwrap_or(0)));
                        }
                    }
                    // The Cloudflare node, whose services an admin switches (`--device cloudflare`).
                    if let Some(parts) = state["network"]["cloudflare_node"]["parts"].as_array() {
                        println!("Cloudflare");
                        for p in parts {
                            println!("  {}  {}", p["name"].as_str().unwrap_or(""), if p["on"] == true { "on" } else { "off" });
                        }
                    }
                }
                // What lets this machine's person start services in their sandbox, for root to run once: `| sudo sh`.
                ["services", "sandbox"] => print!("{}", one::services::sandbox::setup_script()?),
                ["services", "try", dir] => update::try_service(std::path::Path::new(dir), flags.get("device").and_then(Value::as_str))?,
                ["services", how @ ("on" | "off"), name] => {
                    let device = flags.get("device").and_then(Value::as_str);
                    g.node.post("/v1/service", json!({ "name": name, "device": device, "on": *how == "on" }))?;
                    let there = device.map(|s| format!("on {s}")).unwrap_or_else(|| "here".into());
                    match (*how, *name == one::database::scopes::APPS_PART) {
                        _ if device.is_some_and(|d| d.eq_ignore_ascii_case(one::rank::CLOUDFLARE)) => println!("{name} is switched {how} on the Cloudflare node."),
                        ("off", true) => println!("Apps' calls no longer run {there}."),
                        (_, true) => println!("Apps' calls run {there} again."),
                        ("off", _) => println!("{name} is switched off {there}: its oned stops it."),
                        _ => println!("{name} is switched on {there}: its oned sets it up and starts it; {CLI} services says when it's running."),
                    }
                }
                ["schedules"] => {
                    for app in g.node.get("/v1/apps")?.as_array().into_iter().flatten() {
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
                    let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
                    g.node.post("/v1/schedule", json!({ "app": app, "schedule": name, "scope": scope, "on": *on == "on" }))?;
                    println!("{app}'s schedule {name} is {on} in {scope}.");
                }
                ["logs", device] => {
                    takes_only(&flags, &["service", "since", "grep", "lines"])?;
                    let mut input = flags.clone();
                    input.insert("device".into(), json!(device));
                    let read = one::mcp::logs_read(g.node, &input, g.agent, "cli", g.trace);
                    if read.ok.is_err() {
                        tools::print_trace(read.trace.as_deref());
                    }
                    let trace = read.trace;
                    let (lines, note) = one::mcp::logs_said(&read.ok.map_err(|e| anyhow::anyhow!(tools::in_cli_words(g.node, &format!("{e:#}"))))?, device);
                    match g.json {
                        true => println!("{}", serde_json::to_string_pretty(&lines)?),
                        false => {
                            lines.as_array().into_iter().flatten().filter_map(Value::as_str).for_each(|l| println!("{l}"));
                            note.iter().for_each(|n| eprintln!("{n}"));
                        }
                    }
                    tools::print_trace(trace.as_deref());
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Builds: updating every device, releases, and files kept in the network.
fn releases() -> Command {
    Command {
        help: vec![
            ("update".into(), "build main's tip (tested before it was pushed) on this machine and the Macs, and run it everywhere".into()),
            ("release <vX.Y.Z>".into(), "the same, signed off and published on GitHub for the installer".into()),
            ("publish [binary]".into(), "sign and publish a build you made; every device on its platform updates".into()),
            ("file put <file>".into(), "keep a file in the network, for apps and agents; prints its hash".into()),
            ("file get <hash> <file>".into(), "get a kept file from this device or another".into()),
        ],
        run: |g| {
            let flags = g.flags;
            match g.words().as_slice() {
                ["update"] => update::run(None)?,
                ["release", tag] => update::run(Some(tag))?,
                ["publish", rest @ ..] => {
                    let path = match rest {
                        [path] => std::fs::canonicalize(path)?,
                        _ => std::env::current_exe()?,
                    };
                    publish(g.node, &path, &flags, &update::release_key()?)?;
                }
                ["file", "put", file] => {
                    let read = std::fs::File::open(file).with_context(|| format!("read {file}"))?;
                    println!("{}", request(agent().post(format!("http://{}/v1/blobs", api::addr())).send(read))?["blob"].as_str().unwrap_or(""));
                }
                ["file", "get", hash, file] => {
                    let mut resp = agent().get(format!("http://{}/v1/blobs/{hash}", api::addr())).call().map_err(unreached)?;
                    if !resp.status().is_success() {
                        bail!("{}", resp.body_mut().read_json::<Value>()?["error"].as_str().unwrap_or("failed"));
                    }
                    std::fs::write(file, resp.body_mut().with_config().limit(1 << 30).read_to_vec()?)?;
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// This build, and what agents use: skills, search, scripts, MCP and help.
fn agents() -> Command {
    Command {
        help: vec![
            ("version".into(), "this build".into()),
            ("skills [<name> [<file>]]".into(), format!("the skills agents load: {NAME}'s own (how to connect, the rules, which MCP to add; also /llms.txt), and each area's and app's")),
            ("search <words>".into(), "the tools for a task in every app: how to call each, and its app's guide".into()),
            ("run <file>".into(), format!("run a short script that calls tools, as tools.<app>.<tool>({{ ... }}) (- reads it from stdin), and print what it returns; {CLI} search gives each call")),
            ("mcp [--code]".into(), "the local MCP server, over stdio, for agents: every tool, or with --code, search and execute".into()),
            ("help [all]".into(), "what to start with; with all, this".into()),
        ],
        run: |g| {
            let flags = g.flags;
            match g.words().as_slice() {
                [] | ["help"] => {
                    println!("{}", help(g.node));
                    println!("\n{}", connection(g.node));
                }
                ["help", "all"] => println!("{}", every_command()),
                ["help", rest @ ..] => match command_help(g.node, &rest.iter().map(|w| w.to_string()).collect::<Vec<_>>()) {
                    Some(text) => print!("{text}"),
                    None => bail!("{}", tools::unknown(g.node, rest[0], None, Some(g.commands))?),
                },
                ["mcp"] => one::mcp::stdio(std::sync::Arc::new(node()), if flags.contains_key("code") { Mode::Code } else { Mode::All })?,
                ["run", file] => {
                    let script = match *file {
                        "-" => std::io::read_to_string(std::io::stdin())?,
                        file => std::fs::read_to_string(file).with_context(|| format!("read {file}"))?,
                    };
                    let result = code::execute(g.node, &script, g.agent, "cli")?;
                    for line in result["logs"].as_array().into_iter().flatten() {
                        eprintln!("{}", line.as_str().map(String::from).unwrap_or_else(|| line.to_string()));
                    }
                    match result["ok"] == true {
                        true => println!("{}", match &result["value"] {
                            Value::String(s) => s.clone(),
                            v if std::io::IsTerminal::is_terminal(&std::io::stdout()) => serde_json::to_string_pretty(v)?,
                            v => serde_json::to_string(v)?,
                        }),
                        false => bail!("{}", code::shown(&result)),
                    }
                }
                ["skills"] => {
                    let skills = one::mcp::skills(g.node);
                    let width = skills.iter().map(|s| s.name.len()).max().unwrap_or(0);
                    for s in &skills {
                        println!("{:width$}  {}", s.name, s.description);
                    }
                    println!("\n{CLI} skills <name> prints one; {CLI} skills <name> <file> one of its other files.");
                }
                ["skills", name, path @ ..] => {
                    let skill = one::mcp::skills(g.node).into_iter().find(|s| s.name == *name || s.name.strip_prefix(&format!("{}-", one::skills::server_name())) == Some(*name));
                    let skill = skill.with_context(|| format!("no skill {name}: {CLI} skills lists them"))?;
                    let path = path.first().copied().unwrap_or("SKILL.md");
                    print!("{}", skill.file(path).with_context(|| format!("{} has no {path}: it has {}", skill.name, skill.all_files().iter().map(|f| f.0.as_str()).collect::<Vec<_>>().join(", ")))?);
                }
                ["search", words @ ..] if !words.is_empty() => {
                    let limit = flags.get("limit").and_then(|l| l.as_str().and_then(|l| l.parse().ok()).or(l.as_u64().map(|l| l as usize))).unwrap_or(8);
                    let (apps, addable) = code::catalogs(g.node)?;
                    println!("{}", code::search(&apps, &addable, &words.join(" "), limit, code::For::Cli));
                }
                ["version"] if g.json => println!("{}", one::release::Version::this().json()),
                ["version"] => println!("{} {} ({})", NAME, one::config::BUILD, one::config::TARGET),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Any app's tools, after every built-in command.
fn generic() -> Command {
    Command {
        help: vec![
            ("<app>".into(), "an app's tools".into()),
            ("<app> <tool> [words] [--input value]".into(), format!("run a tool, e.g. {CLI} repos list; --help says what it takes. Words fill its main inputs in order (--help shows which), lists repeat a flag, required inputs not given are read as JSON from stdin (a last word - reads it anyway), --file <input>=<path> gives a file and --out <path> saves what it gives back. --device <name> runs it on that device; --trace prints, after its answer (or why it failed), what it did: the devices tried, the one that ran it, each wait in ms, where its data came from, and the total")),
        ],
        run: |g| {
            let args = g.args;
            match g.words().as_slice() {
                [app] if !app.starts_with('-') => {
                    // The node's own apps (Devices, Widgets) aren't in its app list, but read the same.
                    let apps = g.node.get("/v1/apps")?;
                    let own = one::node::own::listed();
                    let Some(found) = apps.as_array().into_iter().flatten().chain(own.iter()).find(|a| a["name"] == *app) else { bail!("{}", tools::unknown(g.node, app, None, Some(g.commands))?) };
                    println!("{} ({app})", found["title"].as_str().unwrap_or(app));
                    if let Some(i) = found["instructions"].as_str().filter(|i| !i.is_empty()) {
                        println!("\n{i}\n");
                    }
                    tools::list(std::slice::from_ref(found)).iter().for_each(|l| println!("{l}"));
                    println!("\n{CLI} {app} <tool> --help says what a tool takes.");
                }
                [app, tool, ..] => {
                    let at = |word: &str, from: usize| args.iter().skip(from).position(|a| a == word).map(|i| i + from + 1).unwrap_or(from);
                    tools::run(g, app, tool, &args[at(tool, at(app, 0))..])?
                }
                [first, ..] => bail!("{}", tools::unknown(g.node, first, None, Some(g.commands))?),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Publishes the binary at `path` as a release, signed with `key`: this one, or one built for another platform
/// (--target <triple> --commit <hash> --committed <seconds>), or a service's program (--service <name>, with those).
fn publish(node: &Client, path: &std::path::Path, flags: &Map<String, Value>, key: &ssh_key::PrivateKey) -> Result<()> {
    // The binary knows what it is; ask it, unless it's for another platform and can't run here, or a service's.
    let service = flags.get("service").and_then(Value::as_str);
    let version = if service.is_none() { one::release::Version::of(path) } else { None };
    let flag = |name: &str| flags.get(name).and_then(Value::as_str).map(str::to_string);
    let version = one::release::Version {
        committed: flags.get("committed").and_then(Value::as_u64).or(version.as_ref().map(|v| v.committed)).context("say when the commit was made with --committed (seconds since 1970)")?,
        commit: flag("commit").or(version.as_ref().map(|v| v.commit.clone())).context("say which commit this is with --commit")?,
        target: flag("target").or(version.as_ref().map(|v| v.target.clone())).context("say which platform this is for with --target")?,
    };
    println!("{}", build::published_said(&build::publish(node, path, key, &version, service)?, service));
    Ok(())
}

/// Deploys the app in `dir`: builds it with its source into a bundle, makes the app if the network hasn't one by that
/// name, and deploys it, saying whether this device runs it yet.
fn deploy(node: &Client, dir: &std::path::Path, flags: &Map<String, Value>) -> Result<()> {
    anyhow::ensure!(dir.is_dir(), "no folder {} in {}: give the app's folder", dir.display(), std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default());
    let name = match flags.get("name").and_then(Value::as_str) {
        Some(name) => name.to_string(),
        None => dir.canonicalize()?.file_name().context("which app?")?.to_string_lossy().to_string(),
    };
    let (blob, service) = build::built_with_service(node, dir)?;
    let here = node.post("/v1/deploy", json!({ "app": name, "bundle": blob, "owner": flags.get("owner"), "wait": true }))?["here"] == true;
    println!("{}", deployed_said(node, &format!("Deployed {name} ({}).{service}", &blob[..12]), here));
    Ok(())
}

/// What a deploy did, after `done`: which devices run it yet, as far as this one knows (`here`, whether it does).
fn deployed_said(node: &Client, done: &str, here: bool) -> String {
    let device = node.get("/v1/state").ok().and_then(|s| s["device"]["name"].as_str().map(str::to_string)).unwrap_or_else(|| "This device".into());
    match here {
        true => format!("{done} {device} runs it now; your other devices take it within a few seconds."),
        false => format!("{done} Your devices take it within a few seconds: until then, its calls say they don't have its latest version yet."),
    }
}

/// A built-in command's help, for --help after it: every_command()'s lines for it, or help() when none are. None for
/// an app's tool, and for an app that isn't a built-in command too, whose help is their own.
fn command_help(node: &Client, words: &[String]) -> Option<String> {
    let first = words.first().filter(|w| *w != "-h")?;
    let text = every_command();
    // The lines for the most words given that have any: vault set's, else every vault line.
    let lines = |prefix: &str| {
        let mut out = String::new();
        let mut taking = false;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix(&format!("  {CLI} {prefix}")) {
                taking = rest.is_empty() || rest.starts_with(' ');
            } else if !line.starts_with("        ") {
                taking = false;
            }
            if taking {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    };
    let commands: Vec<&String> = words.iter().take_while(|w| !w.starts_with('-')).collect();
    let (said, out) = (1..=commands.len()).rev().map(|n| (n, lines(&commands[..n].iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" ")))).find(|(_, o)| !o.is_empty()).unwrap_or_default();
    let app = node.get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default().iter().chain(&one::node::own::listed()).any(|a| a["name"] == first.as_str());
    match (app, out.is_empty()) {
        (true, _) if words.len() > 1 && said < commands.len() => None,
        (true, true) => None,
        (false, true) => Some(format!("{}\n", help(node))),
        (_, false) => Some(format!("Usage:\n{out}")),
    }
}

/// Refuses a flag a command doesn't take, rather than running as if it weren't given: the Vault's, backups' and groups'
/// commands run here, as the person, so `--device` among them.
fn takes_only(flags: &Map<String, Value>, takes: &[&str]) -> Result<()> {
    match flags.keys().find(|k| !takes.contains(&k.as_str())) {
        Some(other) => Err(one::cli::Failed::Typed(format!("--{other} isn't an input of this: see --help")).into()),
        None => Ok(()),
    }
}

/// Splits `--key value` flags from positional words. A flag with no value is true.
fn parse(args: &[String]) -> (Vec<String>, Map<String, Value>) {
    let (mut positional, mut flags) = (Vec::new(), Map::new());
    let mut i = 0;
    while i < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            match args.get(i + 1).filter(|v| !v.starts_with("--")) {
                Some(v) => {
                    let value = serde_json::from_str(v).ok().filter(|v: &Value| !v.is_string()).unwrap_or_else(|| json!(v));
                    flags.insert(key.to_string(), value);
                    i += 2;
                }
                None => {
                    flags.insert(key.to_string(), json!(true));
                    i += 1;
                }
            }
        } else {
            positional.push(args[i].clone());
            i += 1;
        }
    }
    (positional, flags)
}

fn connection(node: &Client) -> String {
    let state = match node.get("/v1/state") {
        Ok(state) => state,
        Err(err) if client_device::active() => return format!("This computer is one of your devices, but {err:#}."),
        Err(_) => return format!("{}.", client_device::not_here()),
    };
    if let Some(said) = client_device::said(&state) {
        return format!("Connected as {said}. {CLI} status shows your devices; {CLI} apps what you can run.");
    }
    let device = state["device"]["name"].as_str().unwrap_or("this device");
    let i = &state["network"];
    if i.is_null() {
        return format!("{device} isn't in {YOURS} yet: {CLI} join <code> joins it, or {CLI} create <name> --as <your name> starts a new one.");
    }
    let network = i["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{YOURS} (still joining)"));
    let me = state["user"]["name"].as_str().or(state["me"]["name"].as_str()).unwrap_or("");
    let ready = if state["device"]["ready"] == true { "" } else { ", catching up" };
    format!("Connected to {network} as {me}, on {device}{ready}. {CLI} status shows its devices; {CLI} apps what you can run.")
}

/// How the CLI asks this machine's node (`agent()`): as whom this machine is signed in as, when `signed_in`.
struct Http {
    signed_in: bool,
}

impl one::api::client::Via for Http {
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, wait: Option<std::time::Duration>) -> Result<one::api::client::Answer> {
        let mut req = ureq::http::Request::builder().method(method).uri(format!("http://{}{path}", api::addr()));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        if self.signed_in && let Some(token) = token() {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let agent = agent();
        let req = agent.configure_request(req.body(body.unwrap_or_default())?).timeout_global(wait).build();
        let resp = agent.run(req).map_err(unreached)?;
        let trace = resp.headers().get(one::node::trace::HEADER).and_then(|h| h.to_str().ok()).map(str::to_string);
        Ok(one::api::client::Answer { status: resp.status().as_u16(), trace, body: Box::new(resp.into_body().into_reader()) })
    }

    fn tries(&self) -> Option<Value> {
        client_device::take_tries()
    }
}

fn node() -> Client {
    Client::new(Http { signed_in: true }, false)
}

/// The access token this machine is signed in with, sent with what it asks its node: none as a device, which asks as its
/// own key, its sign-in only for asking to be let in.
fn token() -> Option<String> {
    if client_device::active() {
        return None;
    }
    one::sign_in::token().unwrap_or_else(|err| {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| eprintln!("{err:#}"));
        None
    })
}

fn to_node<B>(r: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    match token() {
        Some(token) => r.header("authorization", format!("Bearer {token}")),
        None => r,
    }
}

/// An HTTP client for this machine's node, as one of its own programs; or, where none runs and this computer is a
/// device of its person, for their nodes, over iroh (`client_device::instead`).
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).middleware(client_device::instead).middleware(api::as_this_machine).build().into()
}

fn request(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
    // Not even reached: the nodes tried are all there is to say.
    let mut resp = resp.map_err(unreached)?;
    let status = resp.status().as_u16();
    one::api::client::answered(status, resp.body_mut().read_json()?)
}

fn unreached(err: ureq::Error) -> anyhow::Error {
    match client_device::active() {
        true => anyhow::anyhow!("{err}"),
        false => anyhow::Error::new(err).context(client_device::not_here()),
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
        None if !client_device::node_here() => one::sign_in::kept().map(|k| k.address).with_context(|| format!("{NAME} isn't running on this computer, so it signs in as one of your devices: give {YOURS}'s address, {CLI} sign-in --address https://…"))?,
        None => {
            let state = request(agent().get(format!("http://{}/v1/state", api::addr())).call())?;
            state["network"]["settings"]["address"].as_str().context(format!("this network has no address to sign in at yet: give one with --address, or an admin sets it with {CLI} cloudflare use"))?.to_string()
        }
    };
    let join = (!client_device::node_here()).then(|| client_device::signing_in_for(&address)).transpose()?;
    let started = one::sign_in::start(&address, join.as_deref())?;
    let with = if one::config::SIGN_IN == "google" { "Google" } else { "your passkey" };
    // Someone new joins first, in a browser, which then signs this in as them.
    let link = match &invite {
        Some((_, secret)) => {
            let code = started.url.rsplit_once("c=").map_or("", |(_, c)| c);
            let link = format!("{}/invite?code={code}#{secret}", address.trim_end_matches('/'));
            let how = if one::config::SIGN_IN == "google" { "sign in with the Google account it's for" } else { "make your passkey" };
            println!("To join, open this link in a browser on any device, phone or computer, and {how}:\n\n  {link}\n");
            link
        }
        None => {
            println!("To sign in this machine, open this link on any of your devices and sign in with {with}:\n\n  {}\n\nor go to {}/sign-in/code and type {}.", started.url, address.trim_end_matches('/'), started.code);
            started.url.clone()
        }
    };
    if one::sign_in::has_screen() && one::sign_in::open(&link) {
        println!("It's open in your browser here.");
    }
    one::sign_in::finish(started)?;
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
    println!("\nSigned in as {me}: {} is one of your devices now. {CLI} and {CLI} mcp here act as you, on your devices, until {CLI} sign-out.", one::sign_in::hostname());
    Ok(())
}

/// For a node in no network yet, which no one is signed in to: a sign-in this CLI kept for its address was another
/// node's there before, so it isn't sent, and once the node is in a network it's let go.
fn unsigned_post(path: &str, body: Value) -> Result<Value> {
    let r = Client::new(Http { signed_in: false }, false).post(path, body)?;
    let _ = one::sign_in::sign_out();
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
    let apps = node.get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
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

fn ago(ms: u64) -> String {
    let s = one::node::now_ms().saturating_sub(ms) / 1000;
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}
