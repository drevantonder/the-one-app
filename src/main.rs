use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use one::config::{CLI, NAME, YOURS};
use one::cli::{Command, Given};
use one::skills::Mode;
use one::{api, node::Node};
use serde_json::{Map, Value, json};

mod client_device;
mod code;
mod git_remote;
mod mcp_apps;
mod tools;
mod throwaway;
mod update;

/// Every built-in command, each feature's, in the order they're tried: the first that takes what was typed runs it.
fn commands() -> Vec<Command> {
    use one::node::{backups, groups, storage, vault};
    vec![network(), groups::commands(), apps(), vault::cli::commands(), cloudflare_commands(), storage::cli::commands(), backups::cli::commands(), running(), releases(), agents(), throwaway::command(), generic()]
}

/// Every built-in command, with what it takes: `help all` prints it, and `<command> --help` its lines.
fn every_command() -> String {
    format!("Usage:\n{}\n{NAME} holds one network. For another context, run that context's own app.", one::cli::help(&commands()))
}

/// The built-in commands, by their first word, as the full list says them.
fn command_words() -> Vec<String> {
    let mut out: Vec<String> = every_command().lines().filter_map(|l| l.strip_prefix(&format!("  {CLI} "))).filter_map(|l| l.split_whitespace().next()).filter(|w| w.chars().all(|c| c.is_ascii_lowercase() || c == '-')).map(String::from).collect();
    out.dedup();
    out
}

/// What `help` and the bare command say: what an agent needs to start, and where the rest is.
fn help() -> String {
    let apps = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
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

/// `settings` with the network's devices (each key, its roles and whether it's an admin's) and its recovery key, as the
/// node on this machine knows them, for the relay to take only them; none when no node answers here, which `needed`
/// refuses, since that relay would take no one.
fn devices_here(settings: one::cloudflare::Settings, needed: bool) -> Result<one::cloudflare::Settings> {
    let state = get("/v1/state").unwrap_or_default();
    let network = &state["network"];
    let admin = |user: &Value| network["users"].as_array().into_iter().flatten().any(|u| u["id"] == *user && u["role"] == "admin");
    let mut devices: Vec<Value> = ["devices", "clients"]
        .iter()
        .flat_map(|list| network[*list].as_array().into_iter().flatten())
        .filter(|d| d["key"].as_str().is_some_and(|k| k.len() == 64))
        .map(|d| {
            let roles: Vec<&str> = d["roles"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            json!({ "key": d["key"], "role": roles.join(" "), "admin": admin(&d["user"]) })
        })
        .collect();
    anyhow::ensure!(!needed || !devices.is_empty(), "no node of {YOURS} answers on this machine, so its relay would take none of its devices: run this where one does, or with --new for a new network, whose relay takes devices once this runs again beside one");
    // The network's recovery key too, as a client, not an admin's and not a node: what it lets in, the relay takes.
    devices.extend(network["recovery"].as_str().map(|k| json!({ "key": k, "role": "client", "admin": false })));
    let root = network["root"].as_str().filter(|r| r.len() == 64).map(str::to_string);
    Ok(one::cloudflare::Settings { devices, root, ..settings })
}

fn addr() -> String {
    api::addr()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Run as oned: this device, always on; or one of its parts.
    if one::oned::invoked() {
        if let Err(err) = oned(&args) {
            eprintln!("{}: {err:#}", one::oned::name());
            std::process::exit(1);
        }
        return;
    }
    // Run by git, for a repo's address.
    if std::env::args_os().next().is_some_and(|me| std::path::Path::new(&me).file_name().is_some_and(|n| *n == *git_remote::helper())) {
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
        eprintln!("{CLI}: {}", tools::in_cli_words(&format!("{err:#}")));
        std::process::exit(tools::exit_code(&err));
    }
}

fn run(args: Vec<String>) -> Result<()> {
    // --json is everyone's, wherever it's typed: what a tool gives back, as JSON.
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    if args[..end].iter().any(|a| a == "--json") {
        tools::JSON.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // --trace too: a call prints what it did, and where, after its answer.
    let traced = args[..end].iter().any(|a| a == "--trace");
    // (A throwaway's command runs this again with it already taken out.)
    if traced {
        TRACE_ASKED.with(|t| t.set(true));
    }
    // A throwaway's command is run by what it starts (a client's CLI, this again), which takes it out itself.
    let keep_trace = args.first().map(String::as_str) == Some("throwaway");
    let args: Vec<String> = args.iter().enumerate().filter(|(i, a)| *i >= end || (*a != "--json" && (keep_trace || *a != "--trace"))).map(|(_, a)| a.clone()).collect();
    if args.first().map(String::as_str) == Some("throwaway") {
        return throwaway::run(&args[1..]);
    }
    let (positional, flags) = parse(&args);
    if positional.first().map(String::as_str) != Some("mcp") {
        // Printing into `| head` ends quietly, as other commands do, instead of panicking.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    let agent = agent_from_env();
    let given = Given { words: &positional, flags: &flags, args: &args, node: &Local, agent: agent.as_deref() };
    // --help shows help, whatever the command: never runs it. An app's tool's is the app's own.
    if flags.contains_key("help") || positional.iter().any(|p| p == "-h") {
        match command_help(&positional) {
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
                ["sign-in"] => sign_in(&flags)?,
                ["sign-out"] => {
                    // A device takes itself out of its person's devices first, while its sign-in still says who it is; git
                    // here then no longer reaches its repos, which a node here still serves.
                    let address = one::sign_in::kept().map(|k| k.address).filter(|_| !client_device::node_here());
                    let device = client_device::sign_out();
                    if let Some(address) = address {
                        git_remote::unwire(&address);
                    }
                    match one::sign_in::sign_out()? || device {
                        true if device => println!("Signed out: this computer is no longer one of your devices, and {CLI} here no longer acts as you."),
                        true => println!("Signed out: {CLI} on this machine no longer acts as you."),
                        false => println!("This machine isn't signed in."),
                    }
                }
                ["status"] if flags.contains_key("until") => update::wait(&args)?,
                ["status"] => status(&get("/v1/state")?),
                ["create", name @ ..] if !name.is_empty() => {
                    let user_name = flags.get("as").and_then(Value::as_str).context("say what to call you with --as <your name>")?;
                    let r = unsigned_post("/v1/create", json!({ "name": name.join(" "), "user_name": user_name, "user": flags.get("user") }))?;
                    println!("Created {}.\n\nIts recovery key, shown only this once; keep it in your admins' password manager:\n\n  {}\n\nAdd your other machines with: {CLI} pair", name.join(" "), r["recovery"].as_str().unwrap_or(""));
                }
                ["pair"] => {
                    let r = post("/v1/pair", json!({ "user": flags.get("user") }))?;
                    println!("On the other machine, run:\n\n  {CLI} join {}\n\nThe pairing code works once, for seven days.", r["code"].as_str().unwrap_or(""));
                }
                ["join", code] => {
                    unsigned_post("/v1/join", json!({ "code": code }))?;
                    println!("Joined. Syncing now; {CLI} status shows progress.");
                }
                ["settings"] => {
                    let state = get("/v1/state")?;
                    for (name, value) in state["network"]["settings"].as_object().context("not in a network yet")? {
                        println!("{name} = {value}");
                    }
                }
                ["settings", name, value @ ..] => {
                    let value = if flags.contains_key("default") { Value::Null } else { json!(value.join(" ")) };
                    post("/v1/settings", json!({ "name": name, "value": value }))?;
                    println!("Set {name}.");
                }
                ["role", user, role] => {
                    post("/v1/role", json!({ "user": user, "role": role }))?;
                    println!("{user} is now {role}.");
                }
                ["rename", name @ ..] if !name.is_empty() => {
                    post("/v1/name", json!({ "name": name.join(" ") }))?;
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
                    let groups = get("/v1/state")?["network"]["groups"].clone();
                    // Where it's added, as the App Store says it: you, a group by its name, everyone.
                    let place = |scope: &str| match scope.split_once(':') {
                        Some(("group", id)) => groups.as_array().into_iter().flatten().find(|g| g["id"] == id).and_then(|g| g["name"].as_str()).unwrap_or(id).to_string(),
                        Some(("user", _)) => "you".to_string(),
                        _ => "everyone".to_string(),
                    };
                    for app in get("/v1/apps")?.as_array().into_iter().flatten() {
                        let from = app["from"].as_str().map(|f| format!(", a fork of {f}")).unwrap_or_default();
                        let installed: Vec<String> = app["installed"].as_array().into_iter().flatten().filter_map(Value::as_str).map(place).collect();
                        let installed = if installed.is_empty() { String::new() } else { format!(", added for {}", installed.join(", ")) };
                        println!("{} ({}{from}{installed})", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or(""));
                        tools::list(std::slice::from_ref(app)).iter().for_each(|l| println!("{l}"));
                    }
                }
                ["deploy", app] if flags.contains_key("version") => {
                    let version = flags["version"].as_str().context("give --version <hash>")?;
                    let here = post("/v1/deploy", json!({ "app": app, "bundle": version, "wait": true }))?["here"] == true;
                    println!("{}", deployed_said(&format!("Deployed {app} ({version}) again."), here));
                }
                ["deploy", dir] => deploy(std::path::Path::new(dir), &flags)?,
                ["deploys", app] => {
                    let apps = get("/v1/apps")?;
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
                    let r = get(&format!("/v1/source/{app}?{}", q.join("&")))?;
                    match (&r["text"], &r["files"]) {
                        (Value::String(text), _) => print!("{text}"),
                        (_, Value::Array(files)) => files.iter().for_each(|f| println!("{:>8}  {}", f["size"], f["path"].as_str().unwrap_or(""))),
                        _ => bail!("that file isn't text"),
                    }
                }
                ["uninstall", app] => {
                    let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
                    post("/v1/install", json!({ "app": app, "scope": scope, "installed": false }))?;
                    println!("{app} is no longer installed in {scope}.");
                }
                ["fork", app, name] => {
                    post("/v1/fork", json!({ "app": app, "name": name, "owner": flags.get("owner") }))?;
                    println!("Forked {app} as {name}; deploy your changes with {CLI} deploy <dir> --name {name}.");
                }
                ["install", app] => {
                    let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
                    post("/v1/install", json!({ "app": app, "scope": scope, "installed": true }))?;
                    println!("{app} is installed in {scope}.");
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// The network's Workers on Cloudflare, from the device that's their home.
fn cloudflare_commands() -> Command {
    Command {
        help: vec![
            ("cloudflare".into(), "the network's Workers on Cloudflare, as this machine has them: their addresses and settings".into()),
            ("cloudflare serve [--address <url>]".into(), "run the network's Workers in workerd (sign-in, the relay, the URL signer, and the gateway people reach at the network's address) on this machine; the first time, with that address".into()),
            ("cloudflare deploy [--domain <domain>] [--address <url>] [--account <id>] [--new]".into(), format!("deploy the network's Workers to Cloudflare, with an API token that may edit them in CLOUDFLARE_API_TOKEN; the first time, at one of the account's domains ({CLI}-<service>.<domain>, the network's address {CLI}.<domain> unless --address says, for good: passkeys belong to it), or at its workers.dev address; and the Cloudflare node with it, once cloudflare node deploy put it there; beside a node of the network, whose devices its relay takes, unless --new")),
            ("cloudflare invite <name> [--admin] [--user <id>] [--email <email>]".into(), "a link that lets someone join, once and for a few days, as who they'll be in the network (--user, or this machine's person if they're called that; else someone new, as devices invite does), signed with this machine's key, an admin's device's; where people sign in with Google, --email is the account they sign up with. Nobody joins uninvited, the first person too".into()),
            ("cloudflare node deploy [--account <id>]".into(), "put the network's Cloudflare node in its Workers' Cloudflare account (a node as a Worker, with the API token in CLOUDFLARE_API_TOKEN), which devices ask over iroh when none of your machines answers".into()),
            ("cloudflare set <setting> <value>".into(), "change one: sign-in-url, resources (nodes' own addresses, comma-separated), bucket (its JSON object piped, with - as its value) or android (the SHA-256 fingerprints of the certificates the Android app is signed with, comma-separated, or -: links to the address open the app)".into()),
            ("cloudflare use <address>".into(), "point this network at its Workers: its nodes take sign-ins made at the address".into()),
        ],
        run: |g| {
            let flags = g.flags;
            match g.words().as_slice() {
                ["cloudflare"] => print!("{}", cloudflare_said(&one::cloudflare::Settings::load()?)),
                ["cloudflare", "serve"] => {
                    let settings = match one::cloudflare::Settings::load() {
                        Ok(settings) => settings,
                        Err(err) => {
                            one::cloudflare::Settings::none_here().map_err(|here| err.context(format!("{here:#}")))?;
                            let url = flags.get("address").and_then(Value::as_str).context("say where people will reach the network, with --address https://…")?;
                            let settings = one::cloudflare::Settings::new(NAME, url);
                            settings.save()?;
                            settings
                        }
                    };
                    print!("{}", cloudflare_said(&settings));
                    let settings = devices_here(settings, false)?;
                    one::cloudflare::serve(&settings)?
                }
                ["cloudflare", "deploy"] => {
                    let loaded = one::cloudflare::Settings::load();
                    let cf = cloudflare(&flags, loaded.as_ref().ok())?;
                    let settings = match loaded {
                        Ok(settings) => settings,
                        Err(err) => {
                            one::cloudflare::Settings::none_here().map_err(|here| err.context(format!("{here:#}")))?;
                            // Workers already in the account were made elsewhere: new settings here would replace their secrets.
                            let made = cf.scripts()?.contains_key(&one::cloudflare::worker_name("auth"));
                            anyhow::ensure!(!made || flags.contains_key("new"), "the Cloudflare account has this network's Workers already, made on another machine: deploy them there, where their settings are, or bring them here ({} and .env.local). --new replaces them, and everything they hold can't be read again", one::cloudflare::Settings::path().display());
                            let domain = flags.get("domain").and_then(Value::as_str).map(|d| d.trim_matches('.').to_string());
                            let (domain, address) = match domain {
                                Some(domain) => (domain.clone(), format!("https://{CLI}.{domain}")),
                                None => {
                                    let domain = format!("{}.workers.dev", cf.subdomain()?);
                                    (domain.clone(), format!("https://{}.{domain}", one::cloudflare::worker_name("gateway")))
                                }
                            };
                            let at = |service: &str| format!("https://{}.{domain}", one::cloudflare::worker_name(service));
                            let address = flags.get("address").and_then(Value::as_str).map(|a| a.trim_end_matches('/').to_string()).unwrap_or(address);
                            let mut settings = one::cloudflare::Settings::new(NAME, &address);
                            settings.sign_in_url = at("auth");
                            settings.save()?;
                            println!("Keep {} too: the Workers' secrets are in it, and a deploy without them can't read what's there. Better, move each to 1Password and leave an tool(tool://…) reference in its place.\n", one::cloudflare::Settings::path().with_file_name(".env.local").display());
                            // People signing in with Google need its OAuth client, which is put beside the secrets first.
                            match one::config::SIGN_IN {
                                "google" => one::cloudflare::Settings::load()?,
                                _ => settings,
                            }
                        }
                    };
                    let mut settings = settings;
                    at_home(&mut settings)?;
                    let settings = devices_here(settings, !flags.contains_key("new"))?;
                    let modules = one::cloudflare::WORKERS.iter().map(|(_, entry, _)| one::cloudflare::module(entry)).collect::<Result<Vec<_>>>()?;
                    let mut web = settings.web();
                    web.extend(one::cloudflare::precompressed(&web)?);
                    one::cloudflare::deploy(&cf, &settings, &modules, &web)?;
                    if settings.cloudflare_node {
                        one::cloudflare::node::deploy(&cf, &settings)?;
                    }
                    // The Google client its people sign in with, signed by this admin's device: a new device's sign-in counts
                    // only for it (`sign_in_proof::CLIENTS`), whatever sign-in says.
                    if let Some((id, _)) = &settings.google {
                        let clients = format!("https://accounts.google.com {id}");
                        let had = get("/v1/state").ok().and_then(|s| s["network"]["settings"][one::sign_in_proof::CLIENTS].as_str().map(str::to_string));
                        if had.as_deref() != Some(clients.as_str())
                            && let Err(err) = post("/v1/settings", json!({ "name": one::sign_in_proof::CLIENTS, "value": clients }))
                        {
                            eprintln!("A new device's Google sign-in won't let it in by itself until an admin's device runs {CLI} settings {} {clients}: {err:#}", one::sign_in_proof::CLIENTS);
                        }
                    }
                    print!("Deployed.\n{}", cloudflare_said(&settings));
                }
                ["cloudflare", "node", "deploy"] => {
                    let mut settings = one::cloudflare::Settings::load()?;
                    at_home(&mut settings)?;
                    let mut settings = devices_here(settings, true)?;
                    let cf = cloudflare(&flags, Some(&settings))?;
                    // Its endpoint's key, made once and kept with the Workers' secrets.
                    if settings.cloudflare_node_endpoint().is_none() {
                        settings.cloudflare_node_key = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
                        settings.set_secret("CLOUDFLARE_NODE_KEY", &settings.cloudflare_node_key)?;
                    }
                    // The relay it deploys again binds the Cloudflare node's endpoint only for a network that has one.
                    let was = std::mem::replace(&mut settings.cloudflare_node, true);
                    one::cloudflare::node::deploy(&cf, &settings)?;
                    if !was {
                        settings.save()?;
                    }
                    // So devices answer its endpoint, and Devices shows it, for its admins to say where it comes in the order
                    // things run in.
                    post("/v1/settings", json!({ "name": "serverless", "value": settings.cloudflare_node_endpoint() }))?;
                    println!("The Cloudflare node is in your Cloudflare account: devices ask it over iroh when none of your machines answers.");
                }
                ["cloudflare", "invite", name] => {
                    let settings = one::cloudflare::Settings::load()?;
                    // This machine's person, inviting themselves to their own network, is who they are in it.
                    let user = match flags.get("user").and_then(Value::as_str) {
                        Some(user) => Some(user.to_string()),
                        None => get("/v1/state").ok().filter(|s| s["user"]["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))).and_then(|s| s["user"]["id"].as_str().map(str::to_string)),
                    };
                    let email = flags.get("email").and_then(Value::as_str);
                    // Someone new is invited by this machine's node, which keeps the secret for their first device's approval.
                    if user.is_none() && email.is_none() {
                        println!("{}", one::node::devices::app::invited_said(name, &post("/v1/invite-link", json!({ "name": name, "admin": flags.contains_key("admin") }))?));
                    } else {
                        let key = one::node::device_key(&one::home())?;
                        let invite = one::cloudflare::invite(&settings.address, &key, name, user.as_deref(), email)?;
                        let days = one::cloudflare::INVITE_DAYS;
                        let how = if one::config::SIGN_IN == "google" { "they sign in with Google and join" } else { "they make their passkey and join" };
                        println!("Send {name} this link. It works once, for {days} days: opening it, {how}.\n\n  {}", invite.link);
                        // Their first device besides this machine: this machine lets it in.
                        let me = get("/v1/state").ok().and_then(|s| s["user"]["id"].as_str().map(str::to_string));
                        if user.is_some() && user == me {
                            println!("\nIt opens their browser waiting for approval, with a code: approve it here with {CLI} devices approve <code> --confirm.");
                        }
                    }
                }
                ["cloudflare", "set", name, value] => {
                    let mut settings = one::cloudflare::Settings::load()?;
                    let piped = || -> Result<Value> { serde_json::from_reader(std::io::stdin()).context("pipe its JSON object on stdin") };
                    match *name {
                        "sign-in-url" => settings.sign_in_url = value.trim_end_matches('/').to_string(),
                        "resources" => settings.resources = value.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect(),
                        "bucket" if *value == "-" => {
                            let bucket = piped()?;
                            settings.set_secret("BUCKET", &bucket.to_string())?;
                            settings.bucket = Some(bucket);
                        }
                        "bucket" => bail!("pipe its JSON object on stdin and give - as its value, so it's in no shell's history"),
                        "android" => {
                            // As assetlinks.json writes them: 32 pairs of hex digits, in capitals, between colons.
                            let mut prints = Vec::new();
                            for given in value.split(',').map(str::trim).filter(|p| !p.is_empty() && *p != "-") {
                                let hex = given.replace(':', "").to_uppercase();
                                anyhow::ensure!(hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()), "{given} isn't a certificate's SHA-256 fingerprint (apksigner verify --print-certs <apk> says it)");
                                prints.push(hex.as_bytes().chunks(2).map(|p| std::str::from_utf8(p).unwrap()).collect::<Vec<_>>().join(":"));
                            }
                            settings.android = prints;
                        }
                        _ => bail!("no Cloudflare setting {name}: sign-in-url, resources, bucket or android"),
                    }
                    settings.save()?;
                    println!("Set {name}. {CLI} cloudflare serve takes it at its next start, and {CLI} cloudflare deploy when it next runs.");
                }
                ["cloudflare", "use", address] => {
                    post("/v1/settings", json!({ "name": "address", "value": address.trim_end_matches('/') }))?;
                    println!("This network is at {address}: its nodes take sign-ins made there.");
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
                    post("/v1/retire", json!({ "app": app, "how": past }))?;
                    println!("{app} is {past}{}.", if *how == "delete" { ": it's in the App Store's Recently Deleted until you restore or erase it" } else { "" });
                }
                ["services"] => {
                    let state = get("/v1/state")?;
                    let all: Vec<Value> = state["network"]["devices"].as_array().cloned().unwrap_or_default();
                    for s in &all {
                        let installed = s["installed_services"].as_array().cloned().unwrap_or_default();
                        let off = s["off_services"].as_array().cloned().unwrap_or_default();
                        // What a device that says it can run, each on or off, and what it's doing or what went wrong.
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
                    if let Some(parts) = state["network"]["serverless"]["parts"].as_array() {
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
                    post("/v1/service", json!({ "name": name, "device": device, "on": *how == "on" }))?;
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
                    for app in get("/v1/apps")?.as_array().into_iter().flatten() {
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
                    post("/v1/schedule", json!({ "app": app, "schedule": name, "scope": scope, "on": *on == "on" }))?;
                    println!("{app}'s schedule {name} is {on} in {scope}.");
                }
                ["logs", device] => {
                    takes_only(&flags, &["service", "since", "grep", "lines"])?;
                    let mut input = flags.clone();
                    input.insert("device".into(), json!(device));
                    let read = logs_read(&input);
                    if read.is_err() {
                        print_trace();
                    }
                    let (lines, note) = logs_said(&read.map_err(|e| anyhow::anyhow!(tools::in_cli_words(&format!("{e:#}"))))?, device);
                    match tools::json() {
                        true => println!("{}", serde_json::to_string_pretty(&lines)?),
                        false => {
                            lines.as_array().into_iter().flatten().filter_map(Value::as_str).for_each(|l| println!("{l}"));
                            note.iter().for_each(|n| eprintln!("{n}"));
                        }
                    }
                    print_trace();
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
                    publish(&path, &flags, &update::release_key()?)?;
                }
                ["file", "put", file] => {
                    let read = std::fs::File::open(file).with_context(|| format!("read {file}"))?;
                    println!("{}", request(agent().post(format!("http://{}/v1/blobs", addr())).send(read))?["blob"].as_str().unwrap_or(""));
                }
                ["file", "get", hash, file] => {
                    let mut resp = agent().get(format!("http://{}/v1/blobs/{hash}", addr())).call().map_err(unreached)?;
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
                    println!("{}", help());
                    println!("\n{}", connection());
                }
                ["help", "all"] => println!("{}", every_command()),
                ["help", rest @ ..] => match command_help(&rest.iter().map(|w| w.to_string()).collect::<Vec<_>>()) {
                    Some(text) => print!("{text}"),
                    None => bail!("{}", tools::unknown(rest[0], None, true)?),
                },
                ["mcp"] => mcp(if flags.contains_key("code") { Mode::Code } else { Mode::All })?,
                ["run", file] => {
                    let script = match *file {
                        "-" => std::io::read_to_string(std::io::stdin())?,
                        file => std::fs::read_to_string(file).with_context(|| format!("read {file}"))?,
                    };
                    let result = code::execute(&script, agent_from_env().as_deref(), "cli")?;
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
                    let skills = skills();
                    let width = skills.iter().map(|s| s.name.len()).max().unwrap_or(0);
                    for s in &skills {
                        println!("{:width$}  {}", s.name, s.description);
                    }
                    println!("\n{CLI} skills <name> prints one; {CLI} skills <name> <file> one of its other files.");
                }
                ["skills", name, path @ ..] => {
                    let skill = skills().into_iter().find(|s| s.name == *name || s.name.strip_prefix(&format!("{}-", one::skills::server_name())) == Some(*name));
                    let skill = skill.with_context(|| format!("no skill {name}: {CLI} skills lists them"))?;
                    let path = path.first().copied().unwrap_or("SKILL.md");
                    print!("{}", skill.file(path).with_context(|| format!("{} has no {path}: it has {}", skill.name, skill.all_files().iter().map(|f| f.0.as_str()).collect::<Vec<_>>().join(", ")))?);
                }
                ["search", words @ ..] if !words.is_empty() => {
                    let limit = flags.get("limit").and_then(|l| l.as_str().and_then(|l| l.parse().ok()).or(l.as_u64().map(|l| l as usize))).unwrap_or(8);
                    let (apps, addable) = code::catalogs()?;
                    println!("{}", code::search(&apps, &addable, &words.join(" "), limit, code::For::Cli));
                }
                ["version"] if tools::json() => println!("{}", json!({ "commit": one::config::BUILD, "target": one::config::TARGET, "committed": one::config::COMMITTED })),
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
                    let apps = get("/v1/apps")?;
                    let own = one::node::own::listed();
                    let Some(found) = apps.as_array().into_iter().flatten().chain(own.iter()).find(|a| a["name"] == *app) else { bail!("{}", tools::unknown(app, None, true)?) };
                    println!("{} ({app})", found["title"].as_str().unwrap_or(app));
                    if let Some(i) = found["instructions"].as_str().filter(|i| !i.is_empty()) {
                        println!("\n{i}\n");
                    }
                    tools::list(std::slice::from_ref(found)).iter().for_each(|l| println!("{l}"));
                    println!("\n{CLI} {app} <tool> --help says what a tool takes.");
                }
                [app, tool, ..] => {
                    // The words and flags after `<app> <tool>`, as written.
                    let at = |word: &str, from: usize| args.iter().skip(from).position(|a| a == word).map(|i| i + from + 1).unwrap_or(from);
                    tools::run(app, tool, &args[at(tool, at(app, 0))..])?
                }
                [first, ..] => bail!("{}", tools::unknown(first, None, true)?),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Publishes the binary at `path` as a release, signed with `key`: this one, or one built for another platform
/// (--target <triple> --commit <hash> --committed <seconds>), or a service's program (--service <name>, with those).
fn publish(path: &std::path::Path, flags: &Map<String, Value>, key: &ssh_key::PrivateKey) -> Result<()> {
    // The binary knows what it is; ask it, unless it's for another platform and can't run here, or a service's.
    let service = flags.get("service").and_then(Value::as_str);
    let version: Value = (service.is_none()).then(|| std::process::Command::new(path)
        .args(["version", "--json"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice(&o.stdout).ok()))
        .flatten()
        .unwrap_or_default();
    let committed = flags.get("committed").and_then(Value::as_u64).or(version["committed"].as_u64()).context("say when the commit was made with --committed (seconds since 1970)")?;
    let commit = flags.get("commit").and_then(Value::as_str).or(version["commit"].as_str()).context("say which commit this is with --commit")?;
    let target = flags.get("target").and_then(Value::as_str).or(version["target"].as_str()).context("say which platform this is for with --target")?;
    if commit.ends_with("+dirty") {
        bail!("that build has uncommitted changes; commit, build again, then publish");
    }
    let signature = one::release::sign(key, &std::fs::read(path)?)?;
    let body = json!({ "path": path, "commit": commit, "target": target, "committed": committed, "signature": signature, "service": service });
    let r = post(if service.is_some() { "/v1/service-release" } else { "/v1/node-release" }, body)?;
    match service {
        Some(service) => println!("Published {service} {} for {}. Every device on that platform that runs it takes it.", r["commit"].as_str().unwrap_or(""), r["target"].as_str().unwrap_or("")),
        None => println!("Published {} for {}. Every device on that platform updates itself now.", r["commit"].as_str().unwrap_or(""), r["target"].as_str().unwrap_or("")),
    }
    Ok(())
}

/// Deploys the app in `dir`: builds it with its source into a bundle, makes the app if the network hasn't one by that
/// name, and deploys it, saying whether this device runs it yet.
fn deploy(dir: &std::path::Path, flags: &Map<String, Value>) -> Result<()> {
    anyhow::ensure!(dir.is_dir(), "no folder {} in {}: give the app's folder", dir.display(), std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default());
    let name = match flags.get("name").and_then(Value::as_str) {
        Some(name) => name.to_string(),
        None => dir.canonicalize()?.file_name().context("which app?")?.to_string_lossy().to_string(),
    };
    let (blob, service) = built_with_service(dir)?;
    let here = post("/v1/deploy", json!({ "app": name, "bundle": blob, "owner": flags.get("owner"), "wait": true }))?["here"] == true;
    println!("{}", deployed_said(&format!("Deployed {name} ({}).{service}", &blob[..12]), here));
    Ok(())
}

/// The app in `dir` built into a bundle, as a blob of this machine's node, with its service built here for each platform
/// this machine can build it for, each build a blob its bundle names: → the bundle, and what was built of its service.
fn built_with_service(dir: &std::path::Path) -> Result<(String, String)> {
    let mut builds = std::collections::BTreeMap::new();
    let mut service = String::new();
    if let Some((built, missed)) = update::build_service(dir)? {
        for (target, bytes) in &built {
            builds.insert(target.clone(), put_blob(bytes)?);
        }
        let said = |targets: Vec<&str>| targets.into_iter().map(update::platform_said).collect::<Vec<_>>().join(", ");
        service = format!(" Its service is built for {}.", said(built.iter().map(|(t, _)| t.as_str()).collect()));
        if !missed.is_empty() {
            service += &format!(" Not for {}: this machine can't build it for them, so deploy it again from one that can.", said(missed));
        }
    }
    Ok((put_blob(&one::app::deploy::build(dir, &builds)?)?, service))
}

/// Keeps `bytes` as a blob on this machine's node, giving its hash.
fn put_blob(bytes: &[u8]) -> Result<String> {
    Ok(request(agent().post(format!("http://{}/v1/blobs", addr())).send(bytes))?["blob"].as_str().unwrap_or("").to_string())
}

/// The deploy tool's files built into a bundle on this machine, as a blob of its node's: for React screens and a
/// service in Go, which only the machine that deploys builds. → the bundle, and what was built of its service.
fn built_here(files: &Map<String, Value>) -> Result<(String, String)> {
    anyhow::ensure!(!files.contains_key("service/Cargo.toml"), "its service is in Rust, whose build would run its build scripts here: deploy it with {CLI} deploy <its folder>");
    let files = one::app::deploy::given(files)?;
    if !files.iter().any(|(p, _)| p == "service/go.mod") {
        return Ok((put_blob(&one::app::deploy::build_files(&files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect::<Vec<_>>())?)?, String::new()));
    }
    let dir = std::env::temp_dir().join(format!("{CLI}-deploy-{}", std::process::id()));
    let built = (|| {
        for (path, bytes) in &files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap_or(&dir))?;
            std::fs::write(path, bytes)?;
        }
        built_with_service(&dir)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    built
}

/// What a deploy did, after `done`: which devices run it yet, as far as this one knows (`here`, whether it does).
fn deployed_said(done: &str, here: bool) -> String {
    let device = get("/v1/state").ok().and_then(|s| s["device"]["name"].as_str().map(str::to_string)).unwrap_or_else(|| "This device".into());
    match here {
        true => format!("{done} {device} runs it now; your other devices take it within a few seconds."),
        false => format!("{done} Your devices take it within a few seconds: until then, its calls say they don't have its latest version yet."),
    }
}

/// A built-in command's help, for --help after it: every_command()'s lines for it, or help() when none are. None for
/// an app's tool, and for an app that isn't a built-in command too, whose help is their own.
fn command_help(words: &[String]) -> Option<String> {
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
    let app = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default().iter().chain(&one::node::own::listed()).any(|a| a["name"] == first.as_str());
    match (app, out.is_empty()) {
        (true, _) if words.len() > 1 && said < commands.len() => None,
        (true, true) => None,
        (false, true) => Some(format!("{}\n", help())),
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

/// Whether a device runs here, and which install it's in, for the bare command.
fn connection() -> String {
    let state = match get("/v1/state") {
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

/// An HTTP client for this machine's node, as one of its own programs; or, where none runs and this computer is a
/// device of its person, for their nodes, over iroh (`client_device::instead`).
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).middleware(client_device::instead).middleware(api::as_this_machine).build().into()
}

thread_local! {
    /// What the last MCP call that asked for its trace did, as text for its answer.
    static TRACE_TEXT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Whether the calls this thread makes ask their node for a trace (`--trace`, or `trace: true` in MCP), and what the
    /// last one's node said it did (its `x-one-trace`).
    static TRACE_ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TRACED: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

fn request(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
    let mut resp = match resp {
        Ok(resp) => resp,
        // Not even reached: the nodes tried are all there is to say.
        Err(err) => return Err(unreached(err)),
    };
    if let Some(trace) = resp.headers().get(one::node::trace::HEADER).and_then(|h| data_encoding::BASE64.decode(h.as_bytes()).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        TRACED.with(|t| *t.borrow_mut() = Some(trace));
    }
    let refused = matches!(resp.status().as_u16(), 401 | 403);
    let body: Value = resp.body_mut().read_json()?;
    if let Some(err) = body.get("error") {
        if body.get("need").is_some() {
            NEEDED.with(|n| *n.borrow_mut() = Some(body.clone()));
        }
        let err = err.as_str().unwrap_or("failed").to_string();
        let code = body.get("code").and_then(Value::as_str).map(str::to_string);
        return Err(match refused || code.as_deref() == Some(one::protocol::code::REFUSED) {
            true => one::cli::Failed::Refused(err).into(),
            false => one::protocol::Coded::or_plain(code, err),
        });
    }
    Ok(body)
}

/// Why a request to this machine's node, or to its person's as their device, got no answer, saying what to do.
fn unreached(err: ureq::Error) -> anyhow::Error {
    match client_device::active() {
        true => anyhow::anyhow!("{err}"),
        false => anyhow::Error::new(err).context(client_device::not_here()),
    }
}

thread_local! {
    /// The access token of the person an MCP request over HTTP came from, while it's answered (`mcp_http`).
    static SIGNED_IN: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// What a call this thread asked for an MCP request over HTTP needed and wasn't handed (`{ error, need, to, seal }`),
    /// which its caller is answered with 428, to hand them and ask again.
    static NEEDED: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

/// Whether what this thread asks the node, it asks for someone signed in elsewhere, not this machine's person.
fn signed_in() -> bool {
    SIGNED_IN.with(|t| t.borrow().is_some())
}

/// Whether this process is the node, which is never signed in itself: it's only sent its people's access tokens.
static IN_NODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A request to this machine's node: as the person an MCP request over HTTP came from, while it's answered, or as
/// whom this machine is signed in as.
fn to_node<B>(r: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    let token = match SIGNED_IN.with(|t| t.borrow().clone()) {
        Some(token) => Some(token),
        None if IN_NODE.load(std::sync::atomic::Ordering::Relaxed) => None,
        // A device asks as its own key: its sign-in is only for asking to be let in.
        None if client_device::active() => None,
        None => one::sign_in::token().unwrap_or_else(|err| {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| eprintln!("{err:#}"));
            None
        }),
    };
    match token {
        Some(token) => r.header("authorization", format!("Bearer {token}")),
        None => r,
    }
}

/// `sign-in`: this machine signs in at the network's address, as the person who approves its code with their passkey.
fn sign_in(flags: &Map<String, Value>) -> Result<()> {
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
            let state = request(agent().get(format!("http://{}/v1/state", addr())).call())?;
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
    if let Err(err) = git_remote::wire(&address) {
        eprintln!("{err:#}");
    }
    if client_device::node_here() {
        let state = get("/v1/state")?;
        let me = state["user"]["name"].as_str().unwrap_or("you");
        println!("Signed in as {me}. {CLI} and {CLI} mcp on this machine act as you until {CLI} sign-out.");
        return Ok(());
    }
    client_device::join(&address, invite.as_ref().map(|(_, secret)| secret.as_str()))?;
    let state = get("/v1/state")?;
    let me = state["user"]["name"].as_str().unwrap_or("you");
    println!("\nSigned in as {me}: {} is one of your devices now. {CLI} and {CLI} mcp here act as you, on your devices, until {CLI} sign-out.", one::sign_in::hostname());
    Ok(())
}

/// This machine's node, as `get` and `post` ask it, for the node's own apps.
struct Local;

impl one::node::own::Ask for Local {
    fn get(&self, path: &str) -> Result<Value> {
        get(path)
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        post(path, body)
    }
}

fn get(path: &str) -> Result<Value> {
    request(to_node(agent().get(format!("http://{}{path}", addr()))).call())
}

fn post(path: &str, body: Value) -> Result<Value> {
    request(to_node(agent().post(format!("http://{}{path}", addr()))).send_json(&body))
}

/// A call, asking for its trace if `--trace` (or MCP's `trace`) did: what it did stays for `print_trace`.
fn post_call(body: Value) -> Result<Value> {
    let asked = TRACE_ASKED.with(|t| t.get());
    TRACED.with(|t| *t.borrow_mut() = None);
    drop(client_device::take_tries());
    let request_ = to_node(agent().post(format!("http://{}/v1/call", addr())));
    let request_ = if asked { request_.header(one::node::trace::HEADER, "1") } else { request_ };
    request(request_.send_json(&body))
}

/// A device's log, or one of its services', as `logs` asks for it (`device`, `service`, `since`, `grep`, `lines`): the
/// node there says no unless the caller may remove that device (src/node/calls.rs).
fn logs_read(input: &Map<String, Value>) -> Result<Value> {
    let device = input.get("device").and_then(Value::as_str).context("which device? one devices lists them")?;
    let mut asked = input.clone();
    asked.remove("device");
    asked.remove("trace");
    one::logs::Ask::from_json(&Value::Object(asked.clone())).map_err(|e| one::cli::Failed::Typed(format!("{e:#}")))?;
    Ok(post_call(json!({ "device": device, "app": one::database::scopes::APP, "op": "logs", "input": asked, "agent": agent_from_env(), "client": "cli" }))?["ok"].take())
}

/// What a log read gives, as its lines and, if there's something to say about them, a note.
fn logs_said(r: &Value, device: &str) -> (Value, Option<String>) {
    let (shown, matched) = (r["shown"].as_u64().unwrap_or(0), r["matched"].as_u64().unwrap_or(0));
    let note = if shown == 0 {
        let from = r["oldest"].as_u64().map(|ms| format!("; its log goes back to {}", one::node::own::say_time(ms))).unwrap_or_default();
        Some(format!("{device}'s {} log has no lines like that{from}.", r["log"].as_str().unwrap_or("node")))
    } else if matched > shown {
        Some(format!("{shown} of the {matched} lines that match, the newest: narrow it with --since or --grep."))
    } else {
        None
    };
    (r["lines"].clone(), note)
}

/// What the last call did, as lines (`one::node::trace::said`), if it asked for a trace: after its answer, or the
/// reason it failed. Empty if it didn't ask, or made no call.
fn trace_lines() -> Vec<String> {
    if !TRACE_ASKED.with(|t| t.get()) {
        return Vec::new();
    }
    let (node, tries) = (TRACED.with(|t| t.borrow_mut().take()), client_device::take_tries());
    let named = || get("/v1/state").ok().and_then(|s| s["device"]["name"].as_str().map(str::to_string)).unwrap_or_else(|| "the device that answered".into());
    if node.is_none() && tries.is_none() {
        return vec![format!("no trace: this ran in {CLI} here, not on a device")];
    }
    let answered = if node.is_some() { String::new() } else { named() };
    one::node::trace::said(tries.as_ref(), node.as_ref(), &answered)
}

/// Prints a call's trace, after its answer, to stderr, so what's on stdout stays the answer.
fn print_trace() {
    let lines = trace_lines();
    if !lines.is_empty() {
        eprintln!("\ntrace:\n  {}", lines.join("\n  "));
    }
}

/// For a node in no network yet, which no one is signed in to: a sign-in this CLI kept for its address was another
/// node's there before, so it isn't sent, and once the node is in a network it's let go.
fn unsigned_post(path: &str, body: Value) -> Result<Value> {
    let r = request(agent().post(format!("http://{}{path}", addr())).send_json(&body))?;
    let _ = one::sign_in::sign_out();
    Ok(r)
}

fn status(state: &Value) {
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
    println!("\n{}  {} changes", i["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("(joining)"), i["entries"]);
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
    let apps = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
    let title = |app: &str| apps.iter().find(|a| a["name"] == app).and_then(|a| a["title"].as_str()).unwrap_or(app).to_string();
    // Each with a line of the services it runs, and the ones switched off there.
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
    if let Some(priority) = i["serverless"].get("priority") {
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

/// How long ago `ms` (milliseconds since 1970) was, roughly.
fn ago(ms: u64) -> String {
    let s = one::node::now_ms().saturating_sub(ms) / 1000;
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

/// Puts the usual tool folders on PATH: launchd starts the Mac's node with almost none, and the shell, agents and
/// services it starts need Homebrew's and the user's own tools.
fn usual_path() {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let usual = [
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
        "/opt/homebrew/bin".into(),
        "/opt/homebrew/sbin".into(),
        "/usr/local/bin".into(),
        "/usr/local/sbin".into(),
        "/usr/bin".into(),
        "/bin".into(),
        "/usr/sbin".into(),
        "/sbin".into(),
    ];
    // Before what it has, as a login shell puts them (Homebrew's git before the system's).
    let missing: Vec<PathBuf> = usual.into_iter().filter(|d| d.is_dir() && !path.contains(d)).collect();
    path.splice(0..0, missing);
    if let Ok(joined) = std::env::join_paths(path) {
        // Before the runtime starts any thread.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

/// How long a throwaway network's node runs with nobody asking it anything.
const THROWAWAY_IDLE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// When a throwaway network's node was last asked something, in seconds since it started.
static ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// oned: with no part named, this device, for good (`serve`); with one, that part (`service::run`).
fn oned(args: &[String]) -> Result<()> {
    let named: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    match named.as_slice() {
        #[cfg(target_os = "linux")]
        [] if let Some(dir) = args.iter().find_map(|a| a.strip_prefix("--sandboxed=")) => one::services::sandbox::launch(std::path::Path::new(dir)),
        [] => {
            usual_path();
            IN_NODE.store(true, std::sync::atomic::Ordering::Relaxed);
            serve(args.iter().any(|a| a == "--throwaway"), args)
        }
        [name] if *name == one::database::scopes::APPS_PART && let Some(dir) = args.iter().find_map(|a| a.strip_prefix("--runtime=")) => one::app::runtime::run(std::path::Path::new(dir)),
        [name] => one::services::endpoint::run(name),
        _ => bail!("{} runs this device, or with a service's name, that service, as its oned starts it", one::oned::name()),
    }
}

#[tokio::main]
async fn serve(is_try: bool, args: &[String]) -> Result<()> {
    // What it says is kept too, for `one logs`.
    one::logs::keep(one::logs::NODE);
    tracing_subscriber::fmt()
        .with_writer(one::logs::Tee)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into()))
        .init();
    // Asked to stop, even while it starts: its workerd first, then at once, rather than once every blocking task
    // ends, which a stuck request would hold up.
    let stop = stopped()?;
    tokio::spawn(async move {
        stop.await;
        one::app::runtime::stop_all();
        std::process::exit(0)
    });
    let dir = one::home();
    // A new network's first start makes its home.
    std::fs::create_dir_all(&dir).with_context(|| format!("make {}", dir.display()))?;
    // Taking over from the node before, an update's, with its socket: it answers until this one is warm.
    let handed = one::handoff::handed(args)?;
    let listener = match &handed {
        Some(h) => tokio::net::TcpListener::from_std(h.listener.try_clone()?)?,
        None => listen(&dir).await?,
    };
    // Its app runtime is a program of its own, which oned starts as its parts.
    one::app::runtime::APART.store(true, std::sync::atomic::Ordering::SeqCst);
    let node = Node::start(&dir).await?;
    // Its parts that run as processes of their own, as its entries switch them.
    one::oned::keep_parts(node.clone(), tokio::runtime::Handle::current());
    // Git here reaches the network's repos at their address, as on every computer of its person's.
    if !is_try && let Ok(address) = node.address() {
        std::thread::spawn(move || {
            if let Err(err) = git_remote::wire(&address) {
                tracing::warn!("{err:#}");
            }
        });
    }
    if handed.is_some() {
        node.warm().await;
    }
    std::fs::write(dir.join("pid"), std::process::id().to_string())?;
    tracing::info!("{NAME} node {} serving {} from {}", &one::keys::hex(&node.id())[..8], addr(), dir.display());
    tokio::spawn(api::serve_guide(node.clone()));
    api::serve_mcp(axum::routing::post(mcp_http));
    let mut routes = api::guarded(api::routes(), node.clone());
    if is_try {
        let home = dir.clone();
        tokio::spawn(async move {
            if let Err(err) = throwaway::bucket(home).await {
                tracing::warn!("the throwaway network's bucket stopped: {err:#}");
            }
        });
        // Left running, a throwaway network stops by itself; `throwaway` starts it again.
        let started = std::time::Instant::now();
        routes = routes.layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
            ASKED.store(started.elapsed().as_secs(), std::sync::atomic::Ordering::Relaxed);
            next.run(req)
        }));
        let dir = dir.clone();
        tokio::spawn(async move {
            while started.elapsed().as_secs().saturating_sub(ASKED.load(std::sync::atomic::Ordering::Relaxed)) < THROWAWAY_IDLE.as_secs() {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
            tracing::info!("nobody asked the throwaway network anything for {} minutes: stopping", THROWAWAY_IDLE.as_secs() / 60);
            let _ = std::fs::remove_file(dir.join("pid"));
            // Its Workers and browser, then its own group: it keeps its data, for `throwaway` to start it again.
            throwaway::stop_others(&dir);
            unsafe { libc::kill(0, libc::SIGTERM) };
        });
    }
    // Asked to restart (an update, or SIGHUP): the program now at this one's path takes over, and this one stops
    // taking connections, finishes what it's answering, and exits.
    let fd = raw_fd(&listener);
    let (drained, drain) = tokio::sync::oneshot::channel::<()>();
    let restarting = {
        let (node, args) = (node.clone(), args.to_vec());
        async move {
            loop {
                one::handoff::wanted().await;
                tracing::info!("handing over to {}", std::env::current_exe().map(|e| e.display().to_string().trim_end_matches(" (deleted)").to_string()).unwrap_or_default());
                node.handing_over(true).await;
                match one::handoff::hand_over(fd, &args).await {
                    Ok(()) => return node.handed_over(),
                    Err(err) if one::handoff::supervised() => {
                        // Its supervisor starts the new build, as before nodes handed over.
                        tracing::warn!("couldn't hand over ({err:#}): exiting, to be started again");
                        std::process::exit(0);
                    }
                    Err(err) => {
                        tracing::warn!("couldn't hand over, so this build keeps running: {err:#}");
                        node.handing_over(false).await;
                    }
                }
            }
        }
    };
    #[cfg(unix)]
    tokio::spawn(async {
        let Ok(mut hup) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) else { return };
        while hup.recv().await.is_some() {
            let _ = one::handoff::restart();
        }
    });
    tokio::spawn(async move {
        restarting.await;
        let _ = drained.send(());
    });
    let http = api::serve_until(listener, routes, async {
        let _ = drain.await;
    });
    one::handoff::able();
    match handed {
        Some(h) => h.ready(),
        None => one::handoff::notify("READY=1"),
    }
    tracing::info!("{NAME} node {} answering", &one::keys::hex(&node.id())[..8]);
    one::handoff::watch();
    let served = http.await;
    one::app::runtime::stop_all();
    if one::handoff::stopped() {
        std::process::exit(0);
    }
    Ok(served?)
}

#[cfg(unix)]
fn raw_fd(listener: &tokio::net::TcpListener) -> i32 {
    std::os::fd::AsRawFd::as_raw_fd(listener)
}

#[cfg(not(unix))]
fn raw_fd(_listener: &tokio::net::TcpListener) -> i32 {
    -1
}

/// When the node is asked to stop: Ctrl-C, or SIGTERM from its supervisor, an update or a node replacing it, each
/// listened for from when it's called.
fn stopped() -> Result<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (mut term, mut interrupt) = (signal(SignalKind::terminate())?, signal(SignalKind::interrupt())?);
        Ok(Box::pin(async move {
            tokio::select! {
                _ = term.recv() => {}
                _ = interrupt.recv() => {}
            }
        }))
    }
    #[cfg(not(unix))]
    Ok(Box::pin(async {
        let _ = tokio::signal::ctrl_c().await;
    }))
}

/// The node's address, listened on. A node of this network already there (one left running outside launchd or systemd
/// by an earlier build, or started by hand) is stopped first: the one starting is the one to keep, so its supervisor's
/// node never waits on a stray, and an update's node replaces whatever ran before it.
async fn listen(dir: &std::path::Path) -> Result<tokio::net::TcpListener> {
    let bind = || tokio::net::TcpListener::bind(addr());
    match bind().await {
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {}
        bound => return bound.with_context(|| format!("listen on {}", addr())),
    }
    // Who listens there: the pid this home's node wrote, or, for one built before nodes wrote it, the port's listener.
    let port = addr().rsplit(':').next().unwrap_or_default().to_string();
    let mut pids: Vec<i32> = std::fs::read_to_string(dir.join("pid")).ok().and_then(|p| p.trim().parse().ok()).into_iter().collect();
    if let Ok(out) = std::process::Command::new("lsof").args(["-nP", "-t", &format!("-iTCP:{port}"), "-sTCP:LISTEN"]).output() {
        pids.extend(String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse::<i32>().ok()));
    }
    // Only a process of this program, never whatever else took the port.
    let me = std::env::current_exe().ok().and_then(|e| e.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    let ours = |pid: i32| {
        let out = std::process::Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).output().ok();
        out.is_some_and(|o| std::path::Path::new(String::from_utf8_lossy(&o.stdout).trim()).file_name().is_some_and(|n| n.to_string_lossy() == me))
    };
    pids.sort();
    pids.dedup();
    pids.retain(|&pid| pid as u32 != std::process::id() && ours(pid));
    anyhow::ensure!(!pids.is_empty(), "{} is taken, and not by {NAME}'s node", addr());
    for &pid in &pids {
        tracing::warn!("a node of this network was already running (pid {pid}): stopping it, to run this one");
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    for tries in 0..100 {
        if let Ok(listener) = bind().await {
            return Ok(listener);
        }
        if tries == 50 {
            for &pid in &pids {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bind().await.with_context(|| format!("listen on {}: the node there didn't stop", addr()))
}

/// The local MCP server, over stdio: every app tool is a tool, with `search` to find them; or in code mode (`--code`), `search` and
/// `execute`, which runs scripts with them, and the few tools every agent needs from the start.
fn mcp(mode: Mode) -> Result<()> {
    let stdin = std::io::stdin();
    // The agent this serves, credited on its calls: the client's own name for itself, else what its environment says.
    let mut agent = agent_from_env();
    for line in stdin.lock().lines() {
        let Ok(msg) = serde_json::from_str::<Value>(&line?) else { continue };
        // A chat's view waiting on a change is answered beside the rest, which go on meanwhile.
        if msg["method"] == "tools/call" && msg["params"]["name"] == "screens_changes" {
            let mut agent = agent.clone();
            std::thread::spawn(move || {
                if let Some(reply) = mcp_answer(&msg, &mut agent, mode, true) {
                    let mut out = std::io::stdout().lock();
                    let _ = writeln!(out, "{reply}").and_then(|_| out.flush());
                }
            });
            continue;
        }
        if let Some(reply) = mcp_answer(&msg, &mut agent, mode, true) {
            let mut out = std::io::stdout().lock();
            writeln!(out, "{reply}")?;
            if TOOLS_CHANGED.take() {
                writeln!(out, "{}", json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }))?;
            }
            out.flush()?;
        }
    }
    Ok(())
}

/// MCP over Streamable HTTP, at `/mcp`: what `mcp` answers over stdio, for a client device over iroh, as its person (the
/// remote MCP server, cloudflare/mcp/, is remote MCP clients' device), or for a program on this machine, as its person. Each POST
/// is answered with JSON; the server sends nothing unasked, so there's no stream to GET. A tool that needs keys it
/// wasn't handed is answered 428 with what it needs, as `/v1/call` is, for its caller to hand them and ask again.
/// Nothing is kept between requests, since the next may reach another node: the session id carries only the client's
/// name, credited on its calls.
async fn mcp_http(client: Option<axum::Extension<api::FromClient>>, headers: axum::http::HeaderMap, body: String) -> axum::response::Response {
    use axum::response::IntoResponse;
    // A client device's request is asked of the API by a ticket made for it alone, which ends with it.
    let asking = client.map(|axum::Extension(api::FromClient(key, until))| api::AskingAs::new(key, until));
    let token = match &asking {
        Some(asking) => Some(asking.ticket().to_string()),
        None => headers.get("authorization").and_then(|a| a.to_str().ok()).and_then(|a| a.strip_prefix("Bearer ")).map(|t| t.trim().to_string()),
    };
    let mut agent = headers.get("mcp-session-id").and_then(|s| s.to_str().ok()).and_then(|s| s.split_once('.')).and_then(|(_, name)| data_encoding::BASE64URL_NOPAD.decode(name.as_bytes()).ok()).and_then(|n| String::from_utf8(n).ok());
    let Ok(msg) = serde_json::from_str::<Value>(&body) else {
        let error = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "the body isn't JSON" } });
        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(error)).into_response();
    };
    let initialize = msg["method"] == "initialize";
    let answered = tokio::task::spawn_blocking(move || {
        SIGNED_IN.with(|t| *t.borrow_mut() = token);
        NEEDED.with(|n| *n.borrow_mut() = None);
        let reply = match &msg {
            Value::Array(batch) => Some(json!(batch.iter().filter_map(|m| mcp_answer(m, &mut agent, Mode::All, false)).collect::<Vec<_>>())).filter(|r| r.as_array().is_some_and(|r| !r.is_empty())),
            msg => mcp_answer(msg, &mut agent, Mode::All, false),
        };
        SIGNED_IN.with(|t| *t.borrow_mut() = None);
        (reply, agent, NEEDED.with(|n| n.borrow_mut().take()))
    })
    .await;
    drop(asking);
    if let Ok((_, _, Some(needed))) = &answered {
        return (axum::http::StatusCode::PRECONDITION_REQUIRED, axum::Json(needed.clone())).into_response();
    }
    let answered = answered.map(|(reply, agent, _)| (reply, agent));
    match answered {
        Ok((None, _)) => axum::http::StatusCode::ACCEPTED.into_response(),
        Ok((Some(reply), agent)) => {
            let mut resp = axum::Json(reply).into_response();
            if initialize {
                let session = format!("{:016x}.{}", rand::random::<u64>(), data_encoding::BASE64URL_NOPAD.encode(agent.unwrap_or_default().as_bytes()));
                resp.headers_mut().insert("mcp-session-id", session.parse().expect("base64url and hex are header-safe"));
            }
            resp
        }
        Err(err) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({ "error": err.to_string() }))).into_response(),
    }
}

thread_local! {
    /// Whether the call just answered changed the tools a session lists, so stdio tells the client after its answer.
    static TOOLS_CHANGED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Every app's tools by name, as `GET /v1/apps` gives them: what a session's list of tools is made of.
fn tool_names(apps: &Value) -> Vec<String> {
    let mut names: Vec<String> = apps.as_array().into_iter().flatten().flat_map(|a| a["ops"].as_array().into_iter().flatten().map(move |o| format!("{}_{}", a["name"].as_str().unwrap_or(""), o["name"].as_str().unwrap_or("")))).collect();
    names.sort();
    names
}

/// A tool call, and what it changed of the tools a session lists (adding an app, say): over stdio, `stdio` sends MCP's
/// notification after the answer; over HTTP, which sends nothing unasked, the answer says which tools are new.
fn call_and_tell(params: &Value, agent: Option<&str>, mode: Mode, stdio: bool) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let apps = (mode == Mode::All && name != "search").then(|| get("/v1/apps").ok()).flatten();
    let reads = |apps: &Value| match name.split_once('_') {
        Some((a, o)) => one::node::own::find(a).is_some() || apps.as_array().into_iter().flatten().filter(|x| x["name"] == a).flat_map(|x| x["ops"].as_array().into_iter().flatten()).any(|x| x["name"] == o && x["reads"] == true),
        None => name == "search",
    };
    let before = apps.filter(|a| !reads(a)).map(|a| tool_names(&a));
    let mut out = call_tool(params, agent, mode);
    let Some(before) = before else { return out };
    let Ok(after) = get("/v1/apps").map(|a| tool_names(&a)) else { return out };
    if after == before {
        return out;
    }
    if stdio {
        TOOLS_CHANGED.set(true);
        return out;
    }
    let mut new: Vec<&str> = after.iter().filter(|t| !before.contains(t)).filter_map(|t| t.split_once('_').map(|(a, _)| a)).collect();
    new.dedup();
    if let (Some(first), Some(content)) = (new.first(), out["content"].as_array_mut()) {
        content.push(json!({ "type": "text", "text": format!("New tools for {}: they come with your next session. Until then, `{CLI} {first} <tool>` runs them where `{CLI}` runs.", new.join(", ")) }));
    }
    out
}

/// The answer to one MCP message, over stdio or HTTP (`stdio` false): None for a notification. `agent` is the
/// client's name, which `initialize` says.
fn mcp_answer(msg: &Value, agent: &mut Option<String>, mode: Mode, stdio: bool) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => Ok({
            if let Some(name) = msg["params"]["clientInfo"]["name"].as_str().filter(|n| !n.trim().is_empty()) {
                *agent = Some(agent_name(name));
            }
            json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": { "listChanged": stdio }, "resources": {}, "extensions": { "io.modelcontextprotocol/skills": {} } },
            "serverInfo": { "name": CLI, "title": NAME, "version": env!("CARGO_PKG_VERSION") },
            "instructions": instructions(mode),
            })
        }),
        "ping" => Ok(json!({})),
        "tools/list" => tools().map(|t| {
            let mut t = match mode {
                Mode::All => [vec![code::search_tool(mode)], t].concat(),
                Mode::Code => code::code_tools(t),
            };
            t.extend(mcp_apps::tools());
            json!({ "tools": t })
        }),
        "tools/call" => Ok(call_and_tell(&msg["params"], agent.as_deref(), mode, stdio)),
        // Apps' widgets and screens, as MCP Apps (src/mcp_apps.rs).
        "resources/list" => Ok(json!({ "resources": mcp_apps::resources() })),
        "resources/read" => match msg["params"]["uri"].as_str().unwrap_or("") {
            uri if uri.starts_with("skill://") => skill_resource(uri),
            uri => mcp_apps::read(uri),
        },
        // Skills over MCP (SEP-2640): the same skills the agents service writes into harnesses' folders.
        "skills/list" => Ok(json!({ "resultType": "complete", "skills": skills().iter().map(skill_entry).collect::<Vec<_>>(), "ttlMs": 60_000, "cacheScope": "session" })),
        "skills/get" => {
            let uri = msg["params"]["uri"].as_str().unwrap_or("");
            skills().iter().find(|s| format!("skill://{}/SKILL.md", s.name) == uri).map(|s| json!({ "skill": skill_entry(s) })).ok_or_else(|| json!({ "code": -32602, "message": format!("no skill {uri}") }))
        }
        other => Err(json!({ "code": -32601, "message": format!("unknown method {other}") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

/// The skills for this person: the root first (src/skills.rs).
fn skills() -> Vec<one::skills::Skill> {
    let apps = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
    one::skills::all(&apps)
}

/// A skill as `skills/list` lists it: where its SKILL.md is, its frontmatter, and each of its files with its digest.
fn skill_entry(s: &one::skills::Skill) -> Value {
    use sha2::Digest;
    let files: Vec<Value> = s
        .all_files()
        .iter()
        .map(|(path, text)| json!({ "uri": format!("skill://{}/{path}", s.name), "digest": format!("sha256:{}", data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(text.as_bytes()))), "size": text.len() }))
        .collect();
    json!({ "uri": format!("skill://{}/SKILL.md", s.name), "frontmatter": { "name": s.name, "description": s.description }, "resources": files })
}

/// One of a skill's files, as `resources/read` gives it: `skill://<name>/<path>`.
fn skill_resource(uri: &str) -> Result<Value, Value> {
    let (name, path) = uri.trim_start_matches("skill://").split_once('/').unwrap_or((uri, ""));
    let text = skills().into_iter().find(|s| s.name == name).and_then(|s| s.file(path)).ok_or_else(|| json!({ "code": -32002, "message": format!("no resource {uri}") }))?;
    let mime = if path.ends_with(".js") { "text/javascript" } else { "text/markdown" };
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": mime, "text": text }] }))
}

/// The agent running this command, if one is: `<CLI>_AGENT`, else what agent harnesses say about themselves.
fn agent_from_env() -> Option<String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    if let Some(a) = var(&format!("{}_AGENT", CLI.to_uppercase().replace('-', "_"))) {
        return Some(a);
    }
    // AI_AGENT is `<name>_<version>_<kind>`, like claude-code_2-1-314_agent.
    if let Some(a) = var("AI_AGENT") {
        return Some(agent_name(a.split('_').next().unwrap_or(&a)));
    }
    let named = [("CLAUDECODE", "Claude Code"), ("CODEX_THREAD_ID", "Codex"), ("CODEX_SANDBOX", "Codex"), ("OPENCODE", "OpenCode"), ("AMP_THREAD_ID", "Amp")];
    named.iter().find(|(k, _)| var(k).is_some()).map(|(_, n)| n.to_string())
}

/// An agent's name as people know it, from what it calls itself (an MCP client's `clientInfo.name`).
fn agent_name(raw: &str) -> String {
    let known = [
        (&["claude-code", "claude code", "claude-ai", "claude"][..], "Claude Code"),
        (&["codex", "codex-mcp-client", "codex_cli_rs", "codex-cli"][..], "Codex"),
        (&["opencode"][..], "OpenCode"),
        (&["amp", "amp-cli", "ampcode"][..], "Amp"),
        (&["pi", "pi-coding-agent"][..], "Pi"),
        (&["cursor", "cursor-vscode", "cursor-agent"][..], "Cursor"),
        (&["gemini-cli", "gemini"][..], "Gemini CLI"),
        (&["t3-code", "t3code"][..], "T3 Code"),
    ];
    let lower = raw.trim().to_lowercase();
    known.iter().find(|(names, _)| names.contains(&lower.as_str())).map(|(_, n)| n.to_string()).unwrap_or_else(|| raw.trim().to_string())
}

/// The MCP server's instructions (src/guide.rs): the rules first, short enough that every harness keeps all of them.
fn instructions(mode: Mode) -> String {
    let apps = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
    let state = get("/v1/state").unwrap_or_default();
    one::skills::instructions(mode, &apps, state["user"]["name"].as_str())
}

/// Cloudflare's API with the token in CLOUDFLARE_API_TOKEN, or else the Workers' own (their .env.local's), for the account
/// `--account` names or the token's only one.
/// Makes this device the Workers' home, or refuses (exit 3, which `one update` passes over) where it isn't.
fn at_home(settings: &mut one::cloudflare::Settings) -> Result<()> {
    settings.at_home().map_err(|err| match err.downcast::<one::cloudflare::NotHome>() {
        Ok(not) => one::cli::Failed::Refused(not.0).into(),
        Err(err) => err,
    })
}

fn cloudflare(flags: &Map<String, Value>, settings: Option<&one::cloudflare::Settings>) -> Result<one::cloudflare::Cloudflare> {
    let token = std::env::var("CLOUDFLARE_API_TOKEN").ok().or_else(|| settings.and_then(|s| s.cloudflare_token.clone()));
    let token = token.context("give a Cloudflare API token that may edit Workers in CLOUDFLARE_API_TOKEN, or as the Workers' CLOUDFLARE_API_TOKEN in their .env.local")?;
    let api = "https://api.cloudflare.com/client/v4".to_string();
    let account = match flags.get("account").and_then(Value::as_str) {
        Some(account) => account.to_string(),
        None => one::cloudflare::Cloudflare { api: api.clone(), token: token.clone(), account: String::new() }.accounts()?,
    };
    Ok(one::cloudflare::Cloudflare { api, token, account })
}

/// Where a network's Workers are, and what they're set to, without their secrets.
fn cloudflare_said(s: &one::cloudflare::Settings) -> String {
    let mut out = format!("{} on Cloudflare\n  address   {}\n", s.name, s.address);
    if s.sign_in_url.starts_with("http://127.0.0.1") {
        out += &format!("  here on   127.0.0.1:{} (the address), :{} (sign-in)\n", s.ports[1], s.ports[0]);
    }
    if !s.resources.is_empty() {
        out += &format!("  nodes     {}\n", s.resources.join(", "));
    }
    out += &format!("  bucket    {}\n", s.bucket.as_ref().map(|b| format!("{} in {}", b["bucket"].as_str().unwrap_or("?"), b["endpoint"].as_str().unwrap_or("?"))).unwrap_or("none yet: cloudflare set bucket -".into()));
    if !s.android.is_empty() {
        out += &format!("  android   {} signed by {}\n", one::cloudflare::android_package(), s.android.join(", "));
    }
    out
}

/// MCP's `deploy`, `{CLI} deploy` for an agent with no shell: the app's files as text, built where it's asked.
fn deploy_tool() -> Value {
    json!({
        "name": "deploy",
        "description": format!("Deploy an app from its files: builds it as {CLI} deploy builds a folder, npm packages, React screens (screens/<name>.jsx, into dist/<name>.html, with this machine's Bun or Node), a service in Go (service/go.mod, for every platform, with this machine's Go) and all, and makes the app if the name is new, owned by you. Or put back an earlier version. Answers with its tools and how to put it on your page."),
        "inputSchema": { "type": "object", "required": ["app"], "properties": {
            "app": { "type": "string", "description": "the app's name, lowercase words joined by -, like water-log; a new name makes a new app" },
            "files": { "type": "object", "additionalProperties": { "type": "string" }, "description": "every file of the app, each path → its text: { \"app.js\": \"export default { … }\" }, and package.json for npm packages" },
            "version": { "type": "string", "description": "instead of files: an earlier version to put back, from this tool's answer or apps_list" },
        } },
        "annotations": one::skills::hints(false, false),
    })
}

fn logs_tool() -> Value {
    json!({
        "name": "logs",
        "description": "Read a device's node log, or one of its services', over the network, instead of ssh: the newest lines (200 by default, at most 1000 or 48 KiB), oldest first. Only the device's person and the network's admins may.",
        "inputSchema": { "type": "object", "required": ["device"], "properties": {
            "device": { "type": "string", "description": "the device's name (devices_requests and groups_list name them; the status command lists them)" },
            "service": { "type": "string", "description": "a service's name, like whatsapp; omit for the node's own log" },
            "since": { "type": "string", "description": "only the last this long: 30s, 10m, 2h or 1d" },
            "grep": { "type": "string", "description": "only lines with this text in them, whatever its case" },
            "lines": { "type": "integer", "description": "how many of the newest to give" },
        } },
        "annotations": one::skills::hints(true, false),
    })
}

/// What `deploy` did, and what's next: its tools, putting it on a page, and undoing it.
fn deploy_said(app: &str, r: &Value, installed: bool) -> String {
    let short = |b: &Value| b.as_str().unwrap_or("").chars().take(12).collect::<String>();
    let mut out = vec![format!("Deployed {app} ({}){}.", short(&r["bundle"]), if r["made"] == true { ", a new app of yours" } else { "" })];
    let tools: Vec<String> = r["ops"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|o| format!("{app}_{o}")).collect();
    match (&r["failed"], r["here"] == true) {
        (Value::String(why), _) => out.push(format!("{why}. Fix it and deploy again.")),
        (_, false) => out.push("Your devices take it within a few seconds.".into()),
        _ if installed && tools.is_empty() => out.push("It has no tools.".into()),
        _ if installed => out.push(format!("Its tools: {}.", tools.join(", "))),
        _ => out.push(format!(
            "Put it on your page with apps_add {{ app: \"{app}\", scope: \"user\" }}; its tools come then{}, at once where your client takes list_changed, else in your next session.",
            if tools.is_empty() { String::new() } else { format!(" ({})", tools.join(", ")) }
        )),
    }
    out.push(match &r["before"] {
        b if *b == r["bundle"] => "It ran this version already.".into(),
        Value::String(_) => format!("To undo, deploy {{ app: \"{app}\", version: \"{}\" }}.", short(&r["before"])),
        _ => "To undo, apps_delete removes it.".into(),
    });
    out.join("\n")
}

fn tools() -> Result<Vec<Value>, Value> {
    let apps = get("/v1/apps").map_err(|e| json!({ "code": -32000, "message": e.to_string() }))?;
    let mut tools = vec![deploy_tool()];
    // A sign-in from elsewhere can't ask a device for its log.
    if !signed_in() {
        tools.push(logs_tool());
    }
    // What only this machine's person runs, a sign-in from elsewhere can't; and MCP Apps' own version of a tool wins.
    let drawn: Vec<Value> = mcp_apps::tools().iter().map(|t| t["name"].clone()).collect();
    for app in one::node::own::all().into_iter().filter(|a| a.elsewhere || !signed_in()) {
        tools.extend(app.tools.into_iter().filter(|t| !drawn.contains(&t["name"])));
    }
    for app in apps.as_array().into_iter().flatten() {
        let name = app["name"].as_str().unwrap_or("");
        // Tools starting with `_` are for the screens, and the node's own.
        for tool in app["ops"].as_array().into_iter().flatten().filter(|tool| !tool["name"].as_str().unwrap_or("").starts_with('_')) {
            tools.push(json!({ "name": format!("{name}_{}", tool["name"].as_str().unwrap_or("")), "description": tool["description"], "inputSchema": tools::mcp_schema(app, tool), "annotations": tools::mcp_annotations(tool) }));
        }
    }
    Ok(tools)
}

/// A tool's answer, within the budget an agent reads (tools::BUDGET).
fn call_tool(params: &Value, agent: Option<&str>, mode: Mode) -> Value {
    let mut out = answer_tool(params, agent, mode);
    let name = params["name"].as_str().unwrap_or("");
    let chars: usize = out["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).map(str::len).sum();
    if chars > tools::BUDGET {
        let tool = match name {
            "search" => Some(code::search_tool(mode)["inputSchema"].clone()).map(|input| json!({ "input": input })),
            _ => name.split_once('_').and_then(|(a, o)| tools::find(a, o)).map(|(_, o)| o),
        };
        tools::within_budget(&mut out["content"], tool.as_ref());
    }
    out
}

fn answer_tool(params: &Value, agent: Option<&str>, mode: Mode) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let input = params["arguments"].as_object().cloned().unwrap_or_default();
    let failed = |text: String| json!({ "isError": true, "content": [{ "type": "text", "text": text }] });
    // This server's own tools, checked as apps' are, in dispatch.
    let own = [code::search_tool(mode), code::execute_tool(), deploy_tool(), logs_tool()].into_iter().chain(mcp_apps::tools()).find(|t| t["name"] == name);
    if let Some(Err(e)) = own.map(|t| tools::check(&t["inputSchema"], &Value::Object(input.clone()))) {
        return failed(format!("{e:#}"));
    }
    if let Some(out) = mcp_apps::call(name, &Value::Object(input.clone())) {
        return out;
    }
    match name {
        "search" => {
            let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
            match code::catalogs() {
                Ok((apps, addable)) => json!({ "content": [{ "type": "text", "text": code::search(&apps, &addable, input.get("query").and_then(Value::as_str).unwrap_or(""), limit, code::For::Mcp(mode)) }] }),
                Err(e) => failed(format!("{e:#}")),
            }
        }
        "deploy" => {
            let app = input.get("app").and_then(Value::as_str).unwrap_or("");
            let mut service = String::new();
            let body = match (input.get("files"), input.get("version")) {
                // React screens and a service are built here, on the machine that deploys, as `deploy <folder>` builds them.
                (Some(Value::Object(files)), None) if !signed_in() && (one::app::deploy::has_screens(files.keys().map(String::as_str)) || files.keys().any(|p| p.starts_with("service/"))) => match built_here(files) {
                    Ok((bundle, built)) => {
                        service = built;
                        json!({ "app": app, "bundle": bundle, "wait": true })
                    }
                    Err(e) => return failed(format!("{e:#}")),
                },
                (Some(files), None) => json!({ "app": app, "files": files, "wait": true }),
                (None, Some(version)) => json!({ "app": app, "bundle": version, "wait": true }),
                _ => return failed("give files, the app's files, or version, an earlier one to put back".into()),
            };
            match post("/v1/deploy", body) {
                Ok(r) => {
                    let installed = get("/v1/apps").ok().and_then(|a| a.as_array()?.iter().find(|a| a["name"] == app).map(|a| a["installed"].as_array().is_some_and(|i| !i.is_empty())));
                    json!({ "content": [{ "type": "text", "text": format!("{}{service}", deploy_said(app, &r, installed == Some(true))) }] })
                }
                Err(e) => failed(format!("{e:#}")),
            }
        }
        "logs" => match logs_read(&input) {
            Ok(r) => {
                let (lines, note) = logs_said(&r, input.get("device").and_then(Value::as_str).unwrap_or(""));
                let text: Vec<&str> = lines.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                json!({ "content": [{ "type": "text", "text": format!("{}{}", text.join("\n"), note.map(|n| format!("\n{n}")).unwrap_or_default()) }] })
            }
            Err(e) => failed(format!("{e:#}")),
        },
        "execute" => match code::execute(input.get("code").and_then(Value::as_str).unwrap_or(""), agent, "mcp") {
            Ok(result) => json!({ "isError": result["ok"] != true, "content": [{ "type": "text", "text": code::shown(&result) }] }),
            Err(e) => failed(format!("{e:#}")),
        },
        _ => with_trace(match dispatch(name, input, agent, "mcp", None) {
            // A script gets an empty list as it is; an agent reading gets it in words.
            Ok(ok) => match ["accounts", "devices"].into_iter().find_map(|tool| Some((name.strip_suffix(&format!("_{tool}"))?, tool))).filter(|_| ok == json!([])).and_then(|(app, tool)| Some((tools::find(app, tool)?.0, tool))).and_then(|(a, tool)| tools::none_said(&a, tool, &ok, false)) {
                Some(none) => json!({ "content": [{ "type": "text", "text": none }] }),
                None => json!({ "content": tools::mcp_content(&ok) }),
            },
            Err(e) => failed(format!("{e:#}")),
        }),
    }
}

/// A tool's answer with its trace after it, when it asked for one (`trace: true`), as well as when it failed.
fn with_trace(mut out: Value) -> Value {
    if let (Some(text), Some(content)) = (TRACE_TEXT.with(|t| t.borrow_mut().take()), out["content"].as_array_mut()) {
        content.push(json!({ "type": "text", "text": text }));
    }
    out
}

/// What to say for a tool nobody here lists: an app's tools when the app is right, else the nearest tools.
fn unknown_tool(name: &str) -> Result<String> {
    if let Some((app, tool)) = name.split_once('_')
        && get("/v1/apps")?.as_array().into_iter().flatten().chain(&one::node::own::listed()).any(|a| a["name"] == app)
    {
        return tools::unknown(app, Some(tool), false);
    }
    let tools = tools().map_err(|e| anyhow::anyhow!("{}", e["message"].as_str().unwrap_or("")))?;
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    match tools::nearest(name, &names).as_slice() {
        [] => match name.split_once('_') {
            Some((app, tool)) => tools::unknown(app, Some(tool), false),
            None => Ok(format!("no tool {name}: search finds the tools for a task")),
        },
        near => Ok(format!("no tool {name}: did you mean {}?", near.join(" or "))),
    }
}

/// Runs one tool as the person, an app's tool or the Vault's, for MCP or a code mode script: what can't be undone
/// only with confirm: true. `spec` is the tool, when the caller has it already.
fn dispatch(name: &str, mut input: Map<String, Value>, agent: Option<&str>, client: &str, spec: Option<(Value, Value)>) -> Result<Value> {
    let Some((app, tool)) = name.split_once('_') else { bail!("{}", unknown_tool(name)?) };
    let Some((a, spec)) = spec.or_else(|| tools::find(app, tool)) else { bail!("{}", unknown_tool(name)?) };
    // Checked as MCP lists it, before anything runs: a wrong input says what's wrong, never runs as something else.
    tools::check(&tools::mcp_schema(&a, &spec), &Value::Object(input.clone()))?;
    let spec = Some(spec);
    let own = one::node::own::find(app);
    let device = if own.is_some() || spec.as_ref().is_some_and(tools::own_device) { None } else { input.remove("device") };
    let confirmed = input.remove("confirm") == Some(json!(true));
    // `trace: true` gives, beside the answer, what the call did and where (`one <app> <tool> --trace`).
    let traced = input.remove("trace") == Some(json!(true));
    TRACE_TEXT.with(|t| *t.borrow_mut() = None);
    if let Some(spec) = &spec {
        anyhow::ensure!(spec["confirm"] != true || confirmed, "{name} sends or can't be undone, and the person hasn't agreed to it: ask them, and give confirm: true only if they say yes to this very action");
    }
    if let Some(own) = own {
        anyhow::ensure!(own.elsewhere || !signed_in(), "{} is this machine's own person's to use, not someone signed in from elsewhere", own.title);
        return (own.run)(tool, &input, &one::node::own::Asked { node: &Local, agent, client, confirmed });
    }
    if let Some(spec) = spec {
        tools::mcp_files(&spec, &mut input)?;
    }
    if traced {
        TRACE_ASKED.with(|t| t.set(true));
    }
    let called = post_call(json!({ "device": device, "app": app, "op": tool, "input": input, "agent": agent, "client": client }));
    if traced {
        let lines = trace_lines();
        TRACE_ASKED.with(|t| t.set(false));
        TRACE_TEXT.with(|t| *t.borrow_mut() = Some(format!("trace:\n  {}", lines.join("\n  "))));
    }
    Ok(called?["ok"].take())
}
