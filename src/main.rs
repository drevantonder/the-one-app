use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use one::config::{CLI, NAME, YOURS};
use one::skills::Mode;
use one::{api, node::Node};
use serde_json::{Map, Value, json};

mod client_device;
mod code;
mod git_remote;
mod mcp_apps;
mod ops;
mod try_install;
mod update;

/// Every built-in command, with what it takes: `help all` prints it, and `<command> --help` its lines.
fn every_command() -> String {
    format!(
        "Usage:
  {CLI} status                         this device, and the people, groups and devices in {YOURS}
  {CLI} sign-in [--address <url>]      sign this machine in, as you: approve its code by signing in on any of
                                       your devices; {CLI} and {CLI} mcp here then act as you. Where {NAME} doesn't
                                       run, this computer becomes one of your devices, once you approve its next
                                       code too, and asks your devices for everything
  {CLI} sign-in --invite <link>        join with the invite link someone sent you, on your first computer: you make
                                       your passkey in a browser, and they approve this computer's code
  {CLI} sign-out                       sign this machine out, and take it out of your devices if it's one
  {CLI} create <name> --as <your name> [--user <id>]
                                       create an install, with you as its first admin (--user: who you were in one
                                       it replaces, so your sign-in is still yours)
  {CLI} pair [--user <name>]           a pairing code for another of your machines (or, for admins, someone else's)
  {CLI} join <code>                    join {YOURS} with a pairing code
  {CLI} rename <name>                  rename this device
  {CLI} role <user> admin|member       (admins)
  {CLI} settings [<name> <value>|--default]   the install's settings (admins set them)
  {CLI} groups                         who's in {YOURS}, then every group: who's in it, as admin or member,
                                       and who's invited
  {CLI} group create <name>            a group, with you as its admin
  {CLI} group invite <group> <user> [--admin]
  {CLI} group accept <group>
  {CLI} group remove <group> <user>    or leave it, giving yourself
  {CLI} apps                           every app and its operations
  {CLI} deploy <dir> [--name <app>] [--owner <owner>]
                                       build the app in <dir> (app.ts or app.js, and its npm packages) with Bun and
                                       deploy it, with its source; makes the app if it's new (owner: you, a group or admins)
  {CLI} deploy <app> --version <hash>  deploy an earlier version again (deploys lists them): a rollback
  {CLI} deploys <app>                  an app's deploys, newest last
  {CLI} source <app> [path] [--version <hash>]
                                       an app's code as deployed: its files, or one of them
  {CLI} install <app> [--scope user|global|<group>]
                                       install an app, and the apps it uses, for you, a group you admin or everyone
  {CLI} uninstall <app> [--scope s]
  {CLI} fork <app> <name> [--owner <owner>]
                                       a new app from <app>'s code, with data of its own
  {CLI} vault [--scope user|global|<group>]
                                       the Vault's items you may change: fields (secret ones never shown), the sites
                                       they're sent to and the apps switched on for each
  {CLI} vault set <item> <field> [--secret] [--hosts a,b] [--app <app>] [--scope s]
                                       set a field of an item (by title or id; a new one by that title), its value
                                       read from stdin (a line typed, or a whole file); for a person's value, vault
                                       request asks them instead
  {CLI} vault use <item> <app> [--off] switch an app on (or off) for an item; only the person lets agents use one
  {CLI} vault grant <device> [--off] [--scope s]
                                       let a machine open a Vault's secrets and fill them in (or no longer); only
                                       its admins say, and nobody else, not even its people, reads a secret back
  {CLI} vault fetch <url> [--method M] [--headers <json object>] [--body text]
                                       make a request with {{{{field}}}} filled in from the items the person lets
                                       agents use, a secret field only for its item's sites: you get the answer,
                                       never the secret
  {CLI} vault remove <item> [<field>] [--scope s]
                                       delete an item, or one of its fields
  {CLI} vault request [<app>] (--need <name> | --title <t> --fields a,b [--secret-fields b] --hosts a,b) [--why text]
                                       ask the person for an item an app may use (with no app, for vault fetch): a
                                       link to a sheet where they pick or make one, so you never see it; vault status
                                       <id> says how it went
  {CLI} vault requests                 the requests waiting for you; vault answer <id> [--item <id> | --decline]
                                       answers one, a new item's values on stdin (one line, or a JSON object)
  {CLI} kernel                         where this machine's kernel is: its services' addresses and what it's set to
  {CLI} kernel serve [--address <url>]
                                       run the kernel's services (auth, the relay, the URL signer, and the gateway people reach
                                       at the install's address) on this machine; the first time, with that address
  {CLI} kernel deploy [--domain <domain>] [--address <url>] [--account <id>] [--new]
                                       deploy the kernel's services to Cloudflare as Workers, with an API token that
                                       may edit them in CLOUDFLARE_API_TOKEN; the first time, at one of the account's
                                       domains ({CLI}-<service>.<domain>, the install's address {CLI}.<domain> unless
                                       --address says, for good: passkeys belong to it), or at its workers.dev address;
                                       and the serverless node with it, once serverless deploy put it there; beside a
                                       node of the install, whose devices its relay takes, unless --new
  {CLI} kernel invite <name> [--admin] [--user <id>] [--email <email>]
                                       a link that lets someone join, once and for a few days, as who they'll be in the
                                       install (--user, or this machine's person if they're called that; else someone
                                       new, as devices invite does), signed with this machine's key, an admin's
                                       device's; where people sign in with Google, --email is the account they sign up
                                       with. Nobody joins uninvited, the first person too
  {CLI} serverless deploy [--account <id>]
                                       put the install's serverless node in the kernel's Cloudflare account (a node as
                                       a Worker, with the API token in CLOUDFLARE_API_TOKEN), which devices ask over
                                       iroh when none of your machines answers
  {CLI} kernel set <setting> <value>   change one: auth-url, resources (nodes' own addresses, comma-separated), bucket (its JSON object piped, with - as its value) or
                                       android (the SHA-256 fingerprints of the certificates the Android app is signed
                                       with, comma-separated, or -: links to the address open the app)
  {CLI} kernel use <address>
                                       point this install at its kernel: its nodes take sign-ins made at the address
  {CLI} storage                        where the install's data is kept
  {CLI} storage move --endpoint <url> --bucket <name> [--region <r>]
                                       move it to S3-compatible storage (R2, S3, GCS, MinIO), whose key is the
                                       kernel's BUCKET: every device reaches it through the URL signer, and moves there
  {CLI} backups                        how backups are going: each place, its last backup, what's failing
  {CLI} backups now                    back up now, without waiting for the hour
  {CLI} backups place add <folder> [--title t]
                                       back up to a folder on this device, like a USB disk's: tried before it's added
  {CLI} backups place add --endpoint <url> [--title t]
                                       back up to S3-compatible storage (R2, B2, MinIO): piped a JSON object of
                                       bucket, key_id and secret (and region) it tries and adds it; otherwise it asks
                                       the person for them, with a link to a sheet on their device
  {CLI} backups place remove <place>   stop backing up to a place (what's there stays)
  {CLI} backups check [--folder <path>]
                                       restore each place's newest backup into a throwaway copy, check every entry and
                                       blob, and say what came back; with --folder, any backup on a disk, opened with
                                       the recovery key on stdin, with or without an install here
  {CLI} app delete|restore|erase <app> delete an app you own (to Recently Deleted), restore it, or erase it for good
  {CLI} schedules                      every schedule of the apps installed for you: when, as whom, and how it last went
  {CLI} services                       the services on each of your devices: version, and who switched them on
  {CLI} services on|off <name> [--device <name>]
                                       switch a service on or off on a device of yours (here by default), which its
                                       oned starts or stops ({hosted} are in this program; one in another language is
                                       put there first by services/install.sh <name> <device>); apps is its app service
  {CLI} devices keep|stop <app> on <device> [--scope user|global|<group>] [--days <n>]
                                       keep an app running on one of your devices (its schedules run there, with that
                                       app's keys alone, for 30 days and longer as you use it), or stop it
  {CLI} schedule on|off <app> <name> [--scope user|global|<group>]
                                       switch an app's schedule on (it then runs as you) or off
  {CLI} <app>                          an app's operations
  {CLI} <app> <op> [words] [--input value]
                                       run an operation, e.g. {CLI} board post ideas \"hi there\"; --help says what
                                       it takes. Words fill its main inputs in order (--help shows which), lists
                                       repeat a flag, required inputs not given are read as JSON from stdin
                                       (a last word - reads it anyway), --file
                                       <input>=<path> gives a file and --out <path> saves what it gives back.
                                       --device <name> runs it on that device
  {CLI} try [<command>]                a new install for this checkout, on this machine beside yours, with only what
                                       every new install starts with: to look around and test in. {CLI} try <command>
                                       runs a command in it ({CLI} try mcp for an agent), {CLI} try client web, android
                                       or windows gives a client of this checkout's build signed in to it, and
                                       {CLI} try stop deletes it
  {CLI} update                         build main's tip (tested before it was pushed) on this machine and the Macs, and run it everywhere
  {CLI} release <vX.Y.Z>               the same, signed off and published on GitHub for the installer
  {CLI} publish [binary]               sign and publish a build you made; every device on its platform updates
  {CLI} file put <file>                keep a file in the install, for apps and agents; prints its hash
  {CLI} file get <hash> <file>         get a kept file from this device or another
  {CLI} version                        this build
  {CLI} skills [<name> [<file>]]       the skills agents load: {NAME}'s own (how to connect, the rules, which MCP to
                                       add; also {CLI} guide, and /llms.txt), and each area's and app's
  {CLI} search <words>                 the operations for a task in every app: how to call each, and its app's guide
  {CLI} run <file>                     run a short script that calls operations, as tools.<app>.<op>({{ ... }}) (- reads
                                       it from stdin), and print what it returns; {CLI} search gives each call
  {CLI} mcp [--code]                   serve MCP over stdio, for agents: every tool, or with --code, search and execute
  {CLI} help [all]                     what to start with; with all, this

{NAME} holds one install. For another context, run that context's own app.",
        hosted = one::services::hosted().iter().map(|h| h.name).collect::<Vec<_>>().join(", ")
    )
}

/// The built-in commands, by their first word, as the full list says them.
fn commands() -> Vec<String> {
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

  {CLI} search <words>        the operations for a task in every app, each with how to call it
  {CLI} <app>                 an app's operations and its guide
  {CLI} <app> <op> --help     what an operation takes
  {CLI} <app> <op> [words] [--input value]
                             run one, like {CLI} board post ideas \"hi there\"
  {CLI} run <file>            a script that calls several operations; {CLI} search gives each call
  {CLI} skills                the guides agents load
  {CLI} status                who you are, and your devices
  {CLI} mcp [--code]          MCP over stdio, for agents
{yours}
An operation's words fill its main inputs in order, a list repeats its flag, and inputs not given are read as JSON
from stdin. --file <input>=<path> gives a file, --out <path> saves what comes back, --device <name> runs it there,
and --confirm goes ahead with what sends or can't be undone, once the person agreed. --json prints what comes back
as JSON.

Exit codes: 0 done, 2 what was typed is wrong, 3 refused (needs --confirm, not allowed, or not signed in), 1 anything
else. Data goes to stdout, messages to stderr.

{CLI} help all lists every command; {CLI} <command> --help says what one takes."
    )
}

/// `settings` with the install's devices (each key, its roles and whether it's an admin's) and its recovery key, as the
/// node on this machine knows them, for the relay to take only them; none when no node answers here, which `needed`
/// refuses, since that relay would take no one.
fn devices_here(settings: one::kernel::Settings, needed: bool) -> Result<one::kernel::Settings> {
    let state = get("/v1/state").unwrap_or_default();
    let install = &state["install"];
    let admin = |user: &Value| install["users"].as_array().into_iter().flatten().any(|u| u["id"] == *user && u["role"] == "admin");
    let mut devices: Vec<Value> = ["devices", "clients"]
        .iter()
        .flat_map(|list| install[*list].as_array().into_iter().flatten())
        .filter(|d| d["key"].as_str().is_some_and(|k| k.len() == 64))
        .map(|d| {
            let roles: Vec<&str> = d["roles"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            json!({ "key": d["key"], "role": roles.join(" "), "admin": admin(&d["user"]) })
        })
        .collect();
    anyhow::ensure!(!needed || !devices.is_empty(), "no node of your One App answers on this machine, so its relay would take none of its devices: run this where one does, or with --new for a new install, whose relay takes devices once this runs again beside one");
    // The install's recovery key too, as a client, not an admin's and not a node: what it lets in, the relay takes.
    devices.extend(install["recovery"].as_str().map(|k| json!({ "key": k, "role": "client", "admin": false })));
    Ok(one::kernel::Settings { devices, ..settings })
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
        eprintln!("{CLI}: {}", ops::in_cli_words(&format!("{err:#}")));
        std::process::exit(ops::exit_code(&err));
    }
}

fn run(args: Vec<String>) -> Result<()> {
    // --json is everyone's, wherever it's typed: what an operation gives back, as JSON.
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    if args[..end].iter().any(|a| a == "--json") {
        ops::JSON.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let args: Vec<String> = args.iter().enumerate().filter(|(i, a)| *i >= end || *a != "--json").map(|(_, a)| a.clone()).collect();
    if args.first().map(String::as_str) == Some("try") {
        return try_install::try_install(&args[1..]);
    }
    let (positional, flags) = parse(&args);
    if !matches!(positional.first().map(String::as_str), Some("serve" | "mcp")) {
        // Printing into `| head` ends quietly, as other commands do, instead of panicking.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    // --help shows help, whatever the command: never runs it.
    if (flags.contains_key("help") || positional.iter().any(|p| p == "-h"))
        && let Some(text) = command_help(&positional)
    {
        print!("{text}");
        return Ok(());
    }
    match positional.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["help"] => {
            println!("{}", help());
            println!("\n{}", connection());
        }
        ["help", "all"] => println!("{}", every_command()),
        ["help", rest @ ..] => match command_help(&rest.iter().map(|w| w.to_string()).collect::<Vec<_>>()) {
            Some(text) => print!("{text}"),
            None => bail!("{}", ops::unknown(rest[0], None, true)?),
        },
        // Only as a machine moves to oned: a node built before it hands over to this build with `serve`, and its unit
        // still says it (ARCHITECTURE.md's Moving a machine to oned).
        ["serve"] => {
            usual_path();
            IN_NODE.store(true, std::sync::atomic::Ordering::Relaxed);
            serve(flags.contains_key("try"), &args)?
        }
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
        // `guide` was the root skill's name before there were skills.
        ["guide"] => print!("{}", skills()[0].skill_md()),
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
        ["status"] => status(&get("/v1/state")?),
        ["apps"] => {
            let groups = get("/v1/state")?["install"]["groups"].clone();
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
                ops::list(std::slice::from_ref(app)).iter().for_each(|l| println!("{l}"));
            }
        }
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
            for (name, value) in state["install"]["settings"].as_object().context("not in an install yet")? {
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
        ["groups"] | ["group"] | ["groups", "list"] => {
            takes_only(&flags, &[])?;
            print!("{}", groups_said(&get("/v1/state")?));
            println!("\n{CLI} group create <name>, {CLI} group invite <group> <user> [--admin], {CLI} group accept <group> and {CLI} group remove <group> <user> change them.");
        }
        ["group", "create", name @ ..] if !name.is_empty() => {
            let r = post("/v1/group", json!({ "name": name.join(" ") }))?;
            println!("Created {} ({}).", name.join(" "), r["group"].as_str().unwrap_or(""));
        }
        ["group", "invite", group, user] => {
            post("/v1/invite", json!({ "group": group, "user": user, "role": if flags.contains_key("admin") { "admin" } else { "member" } }))?;
            println!("Invited {user}; they join with {CLI} group accept {group}.");
        }
        ["group", "accept", group] => {
            post("/v1/accept", json!({ "group": group }))?;
            println!("You're in {group}.");
        }
        ["group", "remove", group, user] => {
            post("/v1/member", json!({ "group": group, "user": user, "role": null }))?;
            println!("{user} is no longer in {group}.");
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
            post("/v1/install", json!({ "app": app, "scope": scope, "installed": !flags.contains_key("remove") }))?;
            println!("{app} is {} in {scope}.", if flags.contains_key("remove") { "no longer installed" } else { "installed" });
        }
        ["kernel"] => print!("{}", kernel_said(&one::kernel::Settings::load()?)),
        ["kernel", "serve"] => {
            let settings = match one::kernel::Settings::load() {
                Ok(settings) => settings,
                Err(err) => {
                    one::kernel::Settings::none_here().map_err(|here| err.context(format!("{here:#}")))?;
                    let url = flags.get("address").and_then(Value::as_str).context("say where people will reach the install, with --address https://…")?;
                    let settings = one::kernel::Settings::new(NAME, url);
                    settings.save()?;
                    settings
                }
            };
            print!("{}", kernel_said(&settings));
            let settings = devices_here(settings, false)?;
            one::kernel::serve(&settings)?
        }
        ["kernel", "deploy"] => {
            let loaded = one::kernel::Settings::load();
            let cf = cloudflare(&flags, loaded.as_ref().ok())?;
            let settings = match loaded {
                Ok(settings) => settings,
                Err(err) => {
                    one::kernel::Settings::none_here().map_err(|here| err.context(format!("{here:#}")))?;
                    // A kernel already in the account was made elsewhere: new settings here would replace its secrets.
                    let made = cf.scripts()?.contains_key(&one::kernel::worker_name("auth"));
                    anyhow::ensure!(!made || flags.contains_key("new"), "the Cloudflare account has a kernel already, made on another machine: deploy it there, where its settings are, or bring them here ({} and .env.local). --new replaces it, and everything it holds can't be read again", one::kernel::Settings::path().display());
                    let domain = flags.get("domain").and_then(Value::as_str).map(|d| d.trim_matches('.').to_string());
                    let (domain, address) = match domain {
                        Some(domain) => (domain.clone(), format!("https://{CLI}.{domain}")),
                        None => {
                            let domain = format!("{}.workers.dev", cf.subdomain()?);
                            (domain.clone(), format!("https://{}.{domain}", one::kernel::worker_name("gateway")))
                        }
                    };
                    let at = |service: &str| format!("https://{}.{domain}", one::kernel::worker_name(service));
                    let address = flags.get("address").and_then(Value::as_str).map(|a| a.trim_end_matches('/').to_string()).unwrap_or(address);
                    let mut settings = one::kernel::Settings::new(NAME, &address);
                    settings.auth_url = at("auth");
                    settings.save()?;
                    println!("Keep {} too: the kernel's secrets are in it, and a deploy without them can't read what's there. Better, move each to 1Password and leave an op(op://…) reference in its place.\n", one::kernel::Settings::path().with_file_name(".env.local").display());
                    // People signing in with Google need its OAuth client, which is put beside the secrets first.
                    match one::config::SIGN_IN {
                        "google" => one::kernel::Settings::load()?,
                        _ => settings,
                    }
                }
            };
            let settings = devices_here(settings, !flags.contains_key("new"))?;
            let modules = one::kernel::SERVICES.iter().map(|(_, entry, _)| one::kernel::module(entry)).collect::<Result<Vec<_>>>()?;
            let mut web = settings.web();
            web.extend(one::kernel::precompressed(&web)?);
            one::kernel::deploy(&cf, &settings, &modules, &web)?;
            if settings.serverless {
                one::serverless::deploy(&cf, &settings)?;
            }
            print!("Deployed.\n{}", kernel_said(&settings));
        }
        ["serverless", "deploy"] => {
            let mut settings = devices_here(one::kernel::Settings::load()?, true)?;
            let cf = cloudflare(&flags, Some(&settings))?;
            // Its endpoint's key, made once and kept with the kernel's secrets.
            if settings.serverless_endpoint().is_none() {
                settings.serverless_key = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
                settings.set_secret("SERVERLESS_KEY", &settings.serverless_key)?;
            }
            // The relay it deploys again binds the serverless node's endpoint only for an install that has one.
            let was = std::mem::replace(&mut settings.serverless, true);
            one::serverless::deploy(&cf, &settings)?;
            if !was {
                settings.save()?;
            }
            // So devices answer its endpoint, and Devices shows it, for its admins to say where it comes in the order
            // things run in.
            post("/v1/settings", json!({ "name": "serverless", "value": settings.serverless_endpoint() }))?;
            println!("The serverless node is in your Cloudflare account: devices ask it over iroh when none of your machines answers.");
        }
        ["kernel", "invite", name] => {
            let settings = one::kernel::Settings::load()?;
            // This machine's person, inviting themselves to their own install's kernel, is who they are in it.
            let user = match flags.get("user").and_then(Value::as_str) {
                Some(user) => Some(user.to_string()),
                None => get("/v1/state").ok().filter(|s| s["user"]["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))).and_then(|s| s["user"]["id"].as_str().map(str::to_string)),
            };
            let email = flags.get("email").and_then(Value::as_str);
            // Someone new is invited by this machine's node, which keeps the secret for their first device's approval.
            if user.is_none() && email.is_none() {
                println!("{}", invited_said(name, &post("/v1/invite-link", json!({ "name": name, "admin": flags.contains_key("admin") }))?));
            } else {
                let key = one::node::device_key(&one::home())?;
                let invite = one::kernel::invite(&settings.address, &key, name, user.as_deref(), email)?;
                let days = one::kernel::INVITE_DAYS;
                let how = if one::config::SIGN_IN == "google" { "they sign in with Google and join" } else { "they make their passkey and join" };
                println!("Send {name} this link. It works once, for {days} days: opening it, {how}.\n\n  {}", invite.link);
                // Their first device besides this machine: this machine lets it in.
                let me = get("/v1/state").ok().and_then(|s| s["user"]["id"].as_str().map(str::to_string));
                if user.is_some() && user == me {
                    println!("\nIt opens their browser waiting for approval, with a code: approve it here with {CLI} devices approve <code> --confirm.");
                }
            }
        }
        ["kernel", "set", name, value] => {
            let mut settings = one::kernel::Settings::load()?;
            let piped = || -> Result<Value> { serde_json::from_reader(std::io::stdin()).context("pipe its JSON object on stdin") };
            match *name {
                "auth-url" => settings.auth_url = value.trim_end_matches('/').to_string(),
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
                _ => bail!("no kernel setting {name}: auth-url, resources, bucket or android"),
            }
            settings.save()?;
            println!("Set {name}. {CLI} kernel serve takes it at its next start, and {CLI} kernel deploy when it next runs.");
        }
        ["kernel", "use", address] => {
            post("/v1/settings", json!({ "name": "address", "value": address.trim_end_matches('/') }))?;
            println!("This install is at {address}: its nodes take sign-ins made there.");
        }
        ["storage"] => {
            let s = get("/v1/storage")?;
            match s["kind"].as_str() {
                Some("folder") => println!("The install's data is kept in a folder on {}: {}. Move it to S3-compatible storage before other devices join: {CLI} storage move --endpoint <url> --bucket <name>.", s["device"].as_str().unwrap_or(""), s["where"].as_str().unwrap_or("")),
                Some(_) => println!("The install's data is kept in {} at {}.", s["where"].as_str().unwrap_or(""), s["endpoint"].as_str().unwrap_or("")),
                None => println!("This device can't reach the install's data: an admin moves it to S3-compatible storage with {CLI} storage move."),
            }
        }
        ["storage", "move"] => {
            let get = |k: &str| flags.get(k).and_then(Value::as_str);
            let endpoint = get("endpoint").context("give --endpoint, like https://<account>.r2.cloudflarestorage.com")?;
            let bucket = get("bucket").context("give --bucket, the bucket's name: its key is the kernel's (BUCKET, which only the URL signer holds)")?;
            let r = post("/v1/storage", json!({ "endpoint": endpoint, "bucket": bucket, "region": get("region") }))?;
            print!("{}", tried_said(&r));
            if r["kept"] == true {
                println!("Every device moves the install's data there now.");
            }
            if r["moving"] == true {
                println!("This device is putting what it holds there, and keeps the install's data there from now on. Do the same on each of your other devices.");
            }
        }
        ["backups"] => {
            takes_only(&flags, &[])?;
            print!("{}", backups_said(&get("/v1/backups")?));
        }
        ["backups", "now"] => {
            takes_only(&flags, &[])?;
            post("/v1/backups/now", json!({}))?;
            println!("Backing up now. {CLI} backups says how it went.");
        }
        ["backups", "place", "add"] | ["backups", "place", "add", _] => {
            takes_only(&flags, &["title", "endpoint"])?;
            let title = flags.get("title").cloned();
            let r = match (positional.get(3), flags.get("endpoint").and_then(Value::as_str)) {
                (Some(path), None) => post("/v1/backups/place", json!({ "path": std::path::absolute(path)?, "title": title }))?,
                (None, Some(endpoint)) if !std::io::IsTerminal::is_terminal(&std::io::stdin()) => {
                    let mut body: Value = serde_json::from_reader(std::io::stdin()).context("pipe a JSON object of bucket, key_id and secret")?;
                    body["endpoint"] = json!(endpoint);
                    body["title"] = title.unwrap_or(Value::Null);
                    post("/v1/backups/place", body)?
                }
                (None, Some(endpoint)) => {
                    let r = post("/v1/backups/request", json!({ "endpoint": endpoint, "title": title, "agent": agent_from_env() }))?;
                    println!("Open this on your device to give the bucket and its key: {}\nIt's also waiting in Settings, under Requests.", r["link"].as_str().unwrap_or(""));
                    return Ok(());
                }
                _ => anyhow::bail!("give a folder, or --endpoint for S3-compatible storage"),
            };
            print!("{}", tried_said(&r));
            println!("{}", if r["id"].is_string() { "Added: the first backup starts now, then every hour that something changes." } else { "Not added." });
        }
        ["backups", "place", "remove", place] => {
            takes_only(&flags, &["confirm"])?;
            ops::agreed(flags.contains_key("confirm"), &format!("{CLI} backups place remove"))?;
            let places = get("/v1/backups")?["places"].as_array().cloned().unwrap_or_default();
            let p = places.iter().find(|p| p["id"] == *place || p["title"] == *place).with_context(|| format!("no place {place}; {CLI} backups lists them"))?;
            post("/v1/backups/place", json!({ "id": p["id"], "deleted": true }))?;
            println!("No longer backing up to {}. What's there stays.", p["title"].as_str().unwrap_or(""));
        }
        ["backups", "check"] => {
            takes_only(&flags, &["folder"])?;
            let r = match flags.get("folder").and_then(Value::as_str) {
                Some(folder) => check_folder(folder)?,
                None => post("/v1/backups/check", json!({}))?,
            };
            for place in r.as_array().into_iter().flatten() {
                print!("{}", checked_said(place));
            }
        }
        ["vault"] | ["vault", "list"] => {
            takes_only(&flags, &["scope"])?;
            let scope = flags.get("scope").and_then(Value::as_str).map(|s| format!("?scope={s}")).unwrap_or_default();
            for it in get(&format!("/v1/vault{scope}"))?.as_array().into_iter().flatten() {
                println!("{}", item_line(it));
            }
        }
        ["vault", "set", item, field] => {
            takes_only(&flags, &["value", "scope", "hosts", "secret", "app"])?;
            let value = match flags.get("value").and_then(Value::as_str) {
                Some(v) => v.to_string(),
                None => {
                    // Typed, one line; from a file (a key's PEM), all of it.
                    let mut v = String::new();
                    match std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                        true => std::io::stdin().read_line(&mut v)?,
                        false => std::io::Read::read_to_string(&mut std::io::stdin(), &mut v)?,
                    };
                    v.trim_end_matches(['\n', '\r']).to_string()
                }
            };
            anyhow::ensure!(!value.is_empty(), "give the value on stdin");
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let hosts = flags.get("hosts").and_then(Value::as_str).map(|h| h.split(',').map(|h| h.trim().to_string()).collect());
            println!("{}", vault_set(scope, item, field, &value, flags.get("secret").map(|_| true), hosts, flags.get("app").and_then(Value::as_str), agent_from_env().as_deref())?);
        }
        ["vault", "use", item, app] => {
            takes_only(&flags, &["scope", "off"])?;
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            println!("{}", vault_use(scope, item, app, !flags.contains_key("off"), agent_from_env().as_deref())?);
        }
        ["vault", "grant", device] => {
            takes_only(&flags, &["scope", "off"])?;
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            println!("{}", post("/v1/vault/grant", json!({ "scope": scope, "device": device, "on": !flags.contains_key("off") }))?["said"].as_str().unwrap_or(""));
        }
        ["vault", "fetch", url] => {
            takes_only(&flags, &["method", "headers", "body"])?;
            let r = post("/v1/vault/fetch", json!({ "url": url, "method": flags.get("method"), "headers": flags.get("headers"), "body": flags.get("body"), "agent": agent_from_env() }))?;
            eprintln!("{}", r["status"]);
            match r["body"].as_str() {
                Some(body) => println!("{body}"),
                None => println!("{}", r["body_base64"].as_str().unwrap_or("")),
            }
            if let Some(cut) = r["cut"].as_str() {
                eprintln!("({cut})");
            }
        }
        ["vault", "remove", item] | ["vault", "remove", item, _] => {
            takes_only(&flags, &["scope", "confirm"])?;
            ops::agreed(flags.contains_key("confirm"), &format!("{CLI} vault remove"))?;
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            println!("{}", vault_remove(scope, item, positional.get(3).map(String::as_str))?);
        }
        ["vault", "request", app @ ..] if app.len() < 2 => {
            takes_only(&flags, &["scope", "need", "title", "fields", "secret-fields", "hosts", "why"])?;
            // With no app, it's for agents: the person's own, with vault fetch.
            let app = app.first().copied().unwrap_or("agents");
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let list = |k: &str| flags.get(k).and_then(Value::as_str).map(|v| v.split(',').map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
            let secret = list("secret-fields");
            let fields: Map<String, Value> = list("fields").into_iter().map(|f| (f.clone(), json!({ "secret": secret.contains(&f) }))).collect();
            let body = json!({ "scope": scope, "app": app, "need": flags.get("need"), "title": flags.get("title"), "fields": fields, "hosts": list("hosts"), "why": flags.get("why"), "agent": agent_from_env() });
            let r = post("/v1/vault/request", body)?;
            println!("Give the person this link: {}\nTell them; {CLI} vault status {} says how it went once they say they've done it.", r["link"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""));
        }
        ["vault", "status", id] => {
            takes_only(&flags, &[])?;
            let r = get(&format!("/v1/vault/requests?id={id}"))?;
            let r = r.as_array().and_then(|a| a.first()).with_context(|| format!("no request {id}"))?.clone();
            println!("{} for {} in {}: {}", r["title"].as_str().unwrap_or(""), r["app"].as_str().unwrap_or(""), r["scope_name"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""));
        }
        ["vault", "requests"] => {
            takes_only(&flags, &[])?;
            for r in get("/v1/vault/requests")?.as_array().into_iter().flatten() {
                println!("{}  {} for {} in {}  {}  {}", r["id"].as_str().unwrap_or(""), r["title"].as_str().unwrap_or(""), r["app"].as_str().unwrap_or(""), r["scope_name"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""), r["why"].as_str().unwrap_or(""));
            }
        }
        ["vault", "answer", id] => {
            takes_only(&flags, &["scope", "decline", "item"])?;
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let mut body = json!({ "scope": scope, "id": id });
            if flags.contains_key("decline") {
                body["declined"] = json!(true);
            } else if let Some(item) = flags.get("item").and_then(Value::as_str) {
                body["item"] = json!(item);
            } else {
                let r = get(&format!("/v1/vault/requests?id={id}"))?;
                let r = r.as_array().and_then(|a| a.first()).with_context(|| format!("no request {id}"))?.clone();
                let mut text = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
                let text = text.trim_end_matches(['\n', '\r']);
                body["fields"] = match serde_json::from_str::<Value>(text) {
                    Ok(v) if v.is_object() => v,
                    _ => json!({ r["fields"][0]["name"].as_str().unwrap_or(""): text }),
                };
            }
            post("/v1/vault/answer", body)?;
            println!("Answered.");
        }
        ["app", how @ ("delete" | "restore" | "erase"), app] => {
            let past = match *how { "delete" => "deleted", "restore" => "restored", _ => "erased" };
            post("/v1/retire", json!({ "app": app, "how": past }))?;
            println!("{app} is {past}{}.", if *how == "delete" { ": it's in the App Store's Recently Deleted until you restore or erase it" } else { "" });
        }
        ["services"] => {
            let state = get("/v1/state")?;
            let all: Vec<Value> = state["install"]["devices"].as_array().cloned().unwrap_or_default();
            for s in all.iter().filter(|s| s["service"].is_null()) {
                let mut installed = s["installed_services"].as_array().cloned().unwrap_or_default();
                // Its parts with keys of their own, as they say they are.
                installed.extend(all.iter().filter(|p| p["device"] == s["key"]).flat_map(|p| p["installed_services"].as_array().cloned().unwrap_or_default()));
                let off = s["off_services"].as_array().cloned().unwrap_or_default();
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
        }
        ["services", how @ ("on" | "off"), name] => {
            let device = flags.get("device").and_then(Value::as_str);
            post("/v1/service", json!({ "name": name, "device": device, "on": *how == "on" }))?;
            let there = device.map(|s| format!("on {s}")).unwrap_or_else(|| "here".into());
            match (*how, *name == one::members::APPS_PART) {
                ("off", true) => println!("Apps' calls no longer run {there}."),
                (_, true) => println!("Apps' calls run {there} again."),
                ("off", _) => println!("{name} is switched off {there}: its oned stops it."),
                _ => println!("{name} is switched on {there}: its oned starts it, if it's there (a service in another language is put there by services/install.sh {name} in {NAME}'s code)."),
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
        ["devices", op @ ("keep" | "stop"), app, "on", device] => {
            takes_only(&flags, &["scope", "days"])?;
            let mut input = flags.clone();
            input.insert("app".into(), json!(app));
            input.insert("device".into(), json!(device));
            println!("{}", devices_tool(op, &input, "cli")?.as_str().unwrap_or(""));
        }
        ["schedule", on @ ("on" | "off"), app, name] => {
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            post("/v1/schedule", json!({ "app": app, "schedule": name, "scope": scope, "on": *on == "on" }))?;
            println!("{app}'s schedule {name} is {on} in {scope}.");
        }
        ["update"] => update::run(None)?,
        ["release", tag] => update::run(Some(tag))?,
        ["publish", rest @ ..] => {
            let path = match rest {
                [path] => std::fs::canonicalize(path)?,
                _ => std::env::current_exe()?,
            };
            publish(&path, &flags, &update::release_key()?)?;
        }
        // `blob`: what `update` scripts made by builds before `file` call.
        ["file" | "blob", "put", file] => {
            let read = std::fs::File::open(file).with_context(|| format!("read {file}"))?;
            println!("{}", request(agent().post(format!("http://{}/v1/blobs", addr())).send(read))?["blob"].as_str().unwrap_or(""));
        }
        ["file" | "blob", "get", hash, file] => {
            let mut resp = agent().get(format!("http://{}/v1/blobs/{hash}", addr())).call().map_err(unreached)?;
            if !resp.status().is_success() {
                bail!("{}", resp.body_mut().read_json::<Value>()?["error"].as_str().unwrap_or("failed"));
            }
            std::fs::write(file, resp.body_mut().with_config().limit(1 << 30).read_to_vec()?)?;
        }
        ["version"] if ops::json() => println!("{}", json!({ "commit": one::config::BUILD, "target": one::config::TARGET, "committed": one::config::COMMITTED })),
        ["version"] => println!("{} {} ({})", NAME, one::config::BUILD, one::config::TARGET),
        ["rename", name @ ..] if !name.is_empty() => {
            post("/v1/name", json!({ "name": name.join(" ") }))?;
            println!("This device is now {}.", name.join(" "));
        }
        [app] if !app.starts_with('-') => {
            // The node's own apps (Devices, Widgets) aren't in its app list, but read the same.
            let apps = get("/v1/apps")?;
            let own = ops::node_apps();
            let Some(found) = apps.as_array().into_iter().flatten().chain(own.iter()).find(|a| a["name"] == *app) else { bail!("{}", ops::unknown(app, None, true)?) };
            println!("{} ({app})", found["title"].as_str().unwrap_or(app));
            if let Some(i) = found["instructions"].as_str().filter(|i| !i.is_empty()) {
                println!("\n{i}\n");
            }
            ops::list(std::slice::from_ref(found)).iter().for_each(|l| println!("{l}"));
            println!("\n{CLI} {app} <op> --help says what an operation takes.");
        }
        [app, op, ..] => {
            // The words and flags after `<app> <op>`, as written.
            let at = |word: &str, from: usize| args.iter().skip(from).position(|a| a == word).map(|i| i + from + 1).unwrap_or(from);
            ops::run(app, op, &args[at(op, at(app, 0))..])?
        }
        [first, ..] => bail!("{}", ops::unknown(first, None, true)?),
    }
    Ok(())
}

/// Publishes the binary at `path` as a release, signed with `key`: this one, or one built for another platform
/// (--target <triple> --commit <hash> --committed <seconds>).
fn publish(path: &std::path::Path, flags: &Map<String, Value>, key: &ssh_key::PrivateKey) -> Result<()> {
    // The binary knows what it is; ask it, unless it's for another platform and can't run here.
    let version: Value = std::process::Command::new(path)
        .args(["version", "--json"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or_default();
    let committed = flags.get("committed").and_then(Value::as_u64).or(version["committed"].as_u64()).context("say when the commit was made with --committed (seconds since 1970)")?;
    let commit = flags.get("commit").and_then(Value::as_str).or(version["commit"].as_str()).context("say which commit this is with --commit")?;
    let target = flags.get("target").and_then(Value::as_str).or(version["target"].as_str()).context("say which platform this is for with --target")?;
    if commit.ends_with("+dirty") {
        bail!("that build has uncommitted changes; commit, build again, then publish");
    }
    let signature = one::release::sign(key, &std::fs::read(path)?)?;
    let r = post("/v1/node-release", json!({ "path": path, "commit": commit, "target": target, "committed": committed, "signature": signature }))?;
    println!("Published {} for {}. Every device on that platform updates itself now.", r["commit"].as_str().unwrap_or(""), r["target"].as_str().unwrap_or(""));
    Ok(())
}

/// Deploys the app in `dir`: builds it with its source into a bundle, makes the app if the install hasn't one by that
/// name, and deploys it, saying whether this device runs it yet.
fn deploy(dir: &std::path::Path, flags: &Map<String, Value>) -> Result<()> {
    anyhow::ensure!(dir.is_dir(), "no folder {} in {}: give the app's folder", dir.display(), std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default());
    let name = match flags.get("name").and_then(Value::as_str) {
        Some(name) => name.to_string(),
        None => dir.canonicalize()?.file_name().context("which app?")?.to_string_lossy().to_string(),
    };
    let bundle = one::deploy::build(dir)?;
    let blob = request(agent().post(format!("http://{}/v1/blobs", addr())).send(&bundle[..]))?["blob"].as_str().unwrap_or("").to_string();
    let here = post("/v1/deploy", json!({ "app": name, "bundle": blob, "owner": flags.get("owner"), "wait": true }))?["here"] == true;
    println!("{}", deployed_said(&format!("Deployed {name} ({}).", &blob[..12]), here));
    Ok(())
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
/// an app's operation, and for an app that isn't a built-in command too, whose help is their own.
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
    let out = (1..=commands.len()).rev().map(|n| lines(&commands[..n].iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" "))).find(|o| !o.is_empty()).unwrap_or_default();
    let app = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default().iter().any(|a| a["name"] == first.as_str());
    match (app, out.is_empty()) {
        (true, _) if words.len() > 1 => None,
        (true, true) => None,
        (false, true) => Some(format!("{}\n", help())),
        (_, false) => Some(format!("Usage:\n{out}")),
    }
}

/// Refuses a flag a command doesn't take, rather than running as if it weren't given: the Vault's, backups' and groups'
/// commands run here, as the person, so `--device` among them.
fn takes_only(flags: &Map<String, Value>, takes: &[&str]) -> Result<()> {
    match flags.keys().find(|k| !takes.contains(&k.as_str())) {
        Some(other) => Err(ops::Failed::Typed(format!("--{other} isn't an input of this: see --help")).into()),
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
    let i = &state["install"];
    if i.is_null() {
        return format!("{device} isn't in {YOURS} yet: {CLI} join <code> joins it, or {CLI} create <name> --as <your name> starts a new one.");
    }
    let install = i["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{YOURS} (still joining)"));
    let me = state["user"]["name"].as_str().or(state["me"]["name"].as_str()).unwrap_or("");
    let ready = if state["device"]["ready"] == true { "" } else { ", catching up" };
    format!("Connected to {install} as {me}, on {device}{ready}. {CLI} status shows its devices; {CLI} apps what you can run.")
}

/// An HTTP client for this machine's node, as one of its own programs; or, where none runs and this computer is a
/// device of its person, for their nodes, over iroh (`client_device::instead`).
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).middleware(client_device::instead).middleware(api::as_this_machine).build().into()
}

fn request(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
    let mut resp = resp.map_err(unreached)?;
    let refused = matches!(resp.status().as_u16(), 401 | 403);
    let body: Value = resp.body_mut().read_json()?;
    if let Some(err) = body.get("error") {
        if body.get("need").is_some() {
            NEEDED.with(|n| *n.borrow_mut() = Some(body.clone()));
        }
        let err = err.as_str().unwrap_or("failed").to_string();
        return Err(match refused {
            true => ops::Failed::Refused(err).into(),
            false => anyhow::anyhow!(err),
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

/// `sign-in`: this machine signs in to the install's kernel, as the person who approves its code with their passkey.
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
            state["install"]["settings"]["address"].as_str().context(format!("this install has no address to sign in at yet: give one with --address, or an admin sets it with {CLI} kernel use"))?.to_string()
        }
    };
    let started = one::sign_in::start(&address)?;
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

fn get(path: &str) -> Result<Value> {
    request(to_node(agent().get(format!("http://{}{path}", addr()))).call())
}

fn post(path: &str, body: Value) -> Result<Value> {
    request(to_node(agent().post(format!("http://{}{path}", addr()))).send_json(&body))
}

/// For a node in no install yet, which no one is signed in to: a sign-in this CLI kept for its address was another
/// node's there before, so it isn't sent, and once the node is in an install it's let go.
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
    let i = &state["install"];
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
    println!("\nDevices");
    for s in i["devices"].as_array().into_iter().flatten() {
        let dot = if s["online"] == true { "●" } else { "○" };
        // As a device, the node that answered is "here" to itself, not to this computer.
        let here = if s["here"] == true && client_device::me().is_none() { " (this one)" } else { "" };
        let serves = if s["serves_me"] == true { "" } else { ", doesn't run yours" };
        let build = s["build"].as_str().map(|b| format!("  {b}")).unwrap_or_default();
        // A service endpoint is named for its machine; a person reads it as its service on that machine.
        let shown = match s["service"].as_str() {
            Some(service) => format!("{service} on {}", s["name"].as_str().unwrap_or("")),
            None => s["name"].as_str().unwrap_or("").to_string(),
        };
        println!("  {dot} {shown}{here}  {}'s{serves}{build}", name(s["user"].as_str().unwrap_or("")));
    }
    let clients = i["clients"].as_array().cloned().unwrap_or_default();
    if !clients.is_empty() {
        println!("\nPhones");
        for c in &clients {
            // A phone keeps no log and holds no sessions: say when it last asked this device, not that it's offline.
            let asked = c["asked"].as_u64().map(|t| format!(", asked here {}", ago(t))).unwrap_or_default();
            let here = if client_device::me().is_some_and(|(_, key)| c["key"] == key) { " (this one)" } else { "" };
            println!("  ▯ {}{here}  {}{asked}", c["name"].as_str().unwrap_or(""), name(c["user"].as_str().unwrap_or("")));
        }
    }
}

/// The people in the One App, you first, each with their role; then every group in words: its people with their
/// roles, and who's invited.
/// A person or group as groups_list says them: their name, then their handle (`Andre @andre`).
fn handled(v: &Value) -> String {
    match v["handle"].as_str() {
        Some(h) => format!("{} @{h}", v["name"].as_str().unwrap_or("")),
        None => v["name"].as_str().unwrap_or("").to_string(),
    }
}

fn groups_said(state: &Value) -> String {
    let i = &state["install"];
    let names: std::collections::HashMap<&str, String> = i["users"].as_array().into_iter().flatten().filter_map(|u| Some((u["id"].as_str()?, handled(u)))).collect();
    let me = i["user"].as_str();
    let mut users: Vec<&Value> = i["users"].as_array().into_iter().flatten().collect();
    users.sort_by_key(|u| u["me"] != true);
    let everyone: Vec<String> = users.iter().map(|u| {
        let (name, role) = (handled(u), u["role"].as_str().unwrap_or("member"));
        if u["me"] == true { format!("{name} (you, {role})") } else { format!("{name} ({role})") }
    }).collect();
    let mut out = format!("People in {YOURS}: {}.\n", everyone.join(", "));
    let people = |list: &Value| -> Vec<String> {
        list.as_array().into_iter().flatten().map(|m| {
            let id = m["user"].as_str().unwrap_or("");
            let (name, role) = (names.get(id).map_or(id, String::as_str), m["role"].as_str().unwrap_or("member"));
            match me == Some(id) {
                true => format!("{name} (you, {role})"),
                false => format!("{name} ({role})"),
            }
        }).collect()
    };
    let groups = i["groups"].as_array().cloned().unwrap_or_default();
    if groups.is_empty() {
        out.push_str("No groups yet.\n");
        return out;
    }
    out.push_str("Groups:\n");
    for g in &groups {
        out.push_str(&format!("  {}: {}\n", handled(g), people(&g["members"]).join(", ")));
        let invited = people(&g["invited"]);
        if !invited.is_empty() {
            out.push_str(&format!("    invited, not joined yet: {}\n", invited.join(", ")));
        }
    }
    out
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

/// How long a try install's node runs with nobody asking it anything.
const TRY_IDLE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// When a try install's node was last asked something, in seconds since it started.
static ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// oned: with no part named, this device, for good (`serve`); with one, that part (`service::run`).
fn oned(args: &[String]) -> Result<()> {
    let named: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    match named.as_slice() {
        [] => {
            usual_path();
            IN_NODE.store(true, std::sync::atomic::Ordering::Relaxed);
            serve(args.iter().any(|a| a == "--try"), args)
        }
        [name] => one::service::run(name),
        _ => bail!("{} runs this device, or with a service's name, that service, as its oned starts it", one::oned::name()),
    }
}

#[tokio::main]
async fn serve(is_try: bool, args: &[String]) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into()))
        .init();
    // Asked to stop, even while it starts: its workerd first, then at once, rather than once every blocking task
    // ends, which a stuck request would hold up.
    let stop = stopped()?;
    tokio::spawn(async move {
        stop.await;
        one::apps::stop_all();
        std::process::exit(0)
    });
    let dir = one::home();
    // A new install's first start makes its home.
    std::fs::create_dir_all(&dir).with_context(|| format!("make {}", dir.display()))?;
    // Taking over from the node before, an update's, with its socket: it answers until this one is warm.
    let handed = one::handoff::handed(args)?;
    let listener = match &handed {
        Some(h) => tokio::net::TcpListener::from_std(h.listener.try_clone()?)?,
        None => listen(&dir).await?,
    };
    let node = Node::start(&dir, one::services::built_in()).await?;
    // Its parts that run as processes of their own, as its entries switch them.
    one::oned::keep_parts(node.clone(), tokio::runtime::Handle::current());
    // Git here reaches the install's repos at their address, as on every computer of its person's.
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
    tracing::info!("{NAME} node {} serving {} from {}", &one::entry::hex(&node.id())[..8], addr(), dir.display());
    tokio::spawn(api::serve_guide(node.clone()));
    api::serve_mcp(axum::routing::post(mcp_http));
    let mut routes = api::guarded(api::routes(), node.clone());
    if is_try {
        let home = dir.clone();
        tokio::spawn(async move {
            if let Err(err) = try_install::bucket(home).await {
                tracing::warn!("the try install's bucket stopped: {err:#}");
            }
        });
        // Left running, a try install stops by itself; `try` starts it again.
        let started = std::time::Instant::now();
        routes = routes.layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
            ASKED.store(started.elapsed().as_secs(), std::sync::atomic::Ordering::Relaxed);
            next.run(req)
        }));
        let dir = dir.clone();
        tokio::spawn(async move {
            while started.elapsed().as_secs().saturating_sub(ASKED.load(std::sync::atomic::Ordering::Relaxed)) < TRY_IDLE.as_secs() {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
            tracing::info!("nobody asked the try install anything for {} minutes: stopping", TRY_IDLE.as_secs() / 60);
            let _ = std::fs::remove_file(dir.join("pid"));
            // Its kernel and browser, then its own group: it keeps its data, for `try` to start it again.
            try_install::stop_others(&dir);
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
                node.handing_over(true);
                match one::handoff::hand_over(fd, &args).await {
                    Ok(()) => return node.handed_over(),
                    Err(err) if one::handoff::supervised() => {
                        // Its supervisor starts the new build, as before nodes handed over.
                        tracing::warn!("couldn't hand over ({err:#}): exiting, to be started again");
                        std::process::exit(0);
                    }
                    Err(err) => {
                        tracing::warn!("couldn't hand over, so this build keeps running: {err:#}");
                        node.handing_over(false);
                    }
                }
            }
        }
    };
    #[cfg(unix)]
    tokio::spawn(async {
        let Ok(mut hup) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) else { return };
        while hup.recv().await.is_some() {
            one::handoff::restart();
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
    tracing::info!("{NAME} node {} answering", &one::entry::hex(&node.id())[..8]);
    one::handoff::watch();
    let served = http.await;
    one::apps::stop_all();
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

/// The node's address, listened on. A node of this install already there (one left running outside launchd or systemd
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
        tracing::warn!("a node of this install was already running (pid {pid}): stopping it, to run this one");
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

/// MCP over stdio: every app operation is a tool, with `search` to find them; or in code mode (`--code`), `search` and
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
/// MCP Worker, kernel/mcp/, is remote MCP clients' device), or for a program on this machine, as its person. Each POST
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
        Some((a, o)) => ops::node_app(a) || apps.as_array().into_iter().flatten().filter(|x| x["name"] == a).flat_map(|x| x["ops"].as_array().into_iter().flatten()).any(|x| x["name"] == o && x["reads"] == true),
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
        content.push(json!({ "type": "text", "text": format!("New tools for {}: they come with your next session. Until then, `{CLI} {first} <op>` runs them where `{CLI}` runs.", new.join(", ")) }));
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

/// An item, as one line: its title, what it is, its page, its fields (secret ones never shown), where they go and
/// the apps switched on for it.
fn item_line(it: &Value) -> String {
    let fields: Vec<String> = it["fields"].as_array().into_iter().flatten().map(|f| match f["secret"] == true {
        true => format!("{} ••••", f["name"].as_str().unwrap_or("")),
        false => format!("{} = {}", f["name"].as_str().unwrap_or(""), f["value"].as_str().unwrap_or("")),
    }).collect();
    let list = |v: &Value| v.as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
    format!(
        "{}  {}{}  [{}]  {}{}{}",
        it["id"].as_str().unwrap_or(""),
        it["title"].as_str().unwrap_or(""),
        it["type"].as_str().filter(|t| !t.is_empty()).map(|t| format!(" ({t})")).unwrap_or_default(),
        it["scope_name"].as_str().unwrap_or(""),
        fields.join(", "),
        Some(list(&it["hosts"])).filter(|h| !h.is_empty()).map(|h| format!("  → {h}")).unwrap_or_default(),
        Some(list(&it["apps"])).filter(|a| !a.is_empty()).map(|a| format!("  used by: {a}")).unwrap_or_default(),
    )
}

/// An item as an agent reads it: names, not scope ids or pictures, and when it last changed, in ISO 8601.
fn item_said(it: &Value) -> Value {
    let mut out = json!({
        "id": it["id"], "title": it["title"], "page": it["scope_name"], "fields": it["fields"], "hosts": it["hosts"], "apps": it["apps"], "wanted_by": it["wanted_by"],
        "changed": it["time"].as_i64().and_then(|ms| jiff::Timestamp::from_second(ms / 1000).ok()).map(|t| t.to_string()), "by": it["user"],
    });
    for k in ["type", "domain", "app"] {
        if let Some(v) = it[k].as_str().filter(|v| !v.is_empty()) {
            out[k] = json!(v);
        }
    }
    if it["account"] == true {
        out["account"] = json!(true);
    }
    out
}

/// The item `name` (its id or title) names in `scope`, if there's one.
fn find_item(scope: &str, name: &str) -> Result<Option<Value>> {
    let items = get(&format!("/v1/vault?scope={scope}"))?;
    let found: Vec<Value> = items.as_array().into_iter().flatten().filter(|i| i["id"] == name || i["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(name))).cloned().collect();
    anyhow::ensure!(found.len() < 2, "several items are called {name}: give its id");
    Ok(found.into_iter().next())
}

/// Sets `field` of `item` in `scope` (a new item by that title if there's none), secret if asked or if it was.
fn vault_set(scope: &str, item: &str, field: &str, value: &str, secret: Option<bool>, hosts: Option<Vec<String>>, app: Option<&str>, agent: Option<&str>) -> Result<String> {
    let found = find_item(scope, item)?;
    let was = found.as_ref().and_then(|it| it["fields"].as_array()?.iter().find(|f| f["name"] == field).map(|f| f["secret"] == true));
    let mut data = json!({ "scope": scope, "agent": agent, "fields": { field: { "value": value, "secret": secret.or(was).unwrap_or(false) } } });
    if let Some(hosts) = hosts {
        data["hosts"] = json!(hosts);
    }
    let made = match &found {
        Some(it) => {
            data["id"] = it["id"].clone();
            if let Some(app) = app {
                let mut apps: Vec<Value> = it["apps"].as_array().cloned().unwrap_or_default();
                if !apps.contains(&json!(app)) {
                    apps.push(json!(app));
                }
                data["apps"] = json!(apps);
            }
            false
        }
        None => {
            data["title"] = json!(item);
            data["apps"] = json!(app.into_iter().collect::<Vec<_>>());
            true
        }
    };
    let id = post("/v1/vault", data)?["id"].as_str().unwrap_or("").to_string();
    let done = match made {
        true => format!("Made {item} ({id}) in {}, with {field}.", vault_name(scope)),
        false => format!("Set {item}'s {field}."),
    };
    // Whether an agent may use it now, since only the person lets agents use an item.
    let agents = found.as_ref().is_some_and(|it| it["apps"].as_array().is_some_and(|a| a.iter().any(|a| a == one::agents::NAME))) || app == Some(one::agents::NAME);
    Ok(match (agents, app) {
        (true, _) => format!("{done} Agents use it with vault_fetch, as {{{{{id}:{field}}}}}."),
        (false, None) => format!("{done} Agents may not use it yet: vault_request, leaving out app, asks the person to let you, and they pick it."),
        (false, Some(_)) => done,
    })
}

/// Switches `app` on or off for `item` in `scope`.
fn vault_use(scope: &str, item: &str, app: &str, on: bool, agent: Option<&str>) -> Result<String> {
    let it = find_item(scope, item)?.with_context(|| format!("no item {item} in {scope}"))?;
    let mut apps: Vec<Value> = it["apps"].as_array().cloned().unwrap_or_default().into_iter().filter(|a| a != app).collect();
    if on {
        apps.push(json!(app));
    }
    post("/v1/vault", json!({ "scope": scope, "id": it["id"], "apps": apps, "agent": agent }))?;
    Ok(format!("{app} is switched {} for {}.", if on { "on" } else { "off" }, it["title"].as_str().unwrap_or(item)))
}

/// A scope's Vault as a person calls it: your Vault, Everyone's Vault, <name>'s Vault.
fn vault_name(scope: &str) -> String {
    match scope {
        "user" => "your Vault".to_string(),
        "global" => "Everyone's Vault".to_string(),
        group => format!("{group}'s Vault"),
    }
}

/// Deletes `item` in `scope`, or with `field`, one of its fields. When it's not there but is in another Vault you
/// may change, says which and gives the command that removes it.
fn vault_remove(scope: &str, item: &str, field: Option<&str>) -> Result<String> {
    let it = match find_item(scope, item)? {
        Some(it) => it,
        None => {
            let all = get("/v1/vault")?.as_array().cloned().unwrap_or_default();
            let where_it: Vec<String> = all
                .iter()
                .filter(|i| i["id"] == item || i["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(item)))
                .map(|it| {
                    let s = it["scope"].as_str().unwrap_or("");
                    let (flag, vault) = match s {
                        s if s.starts_with("user:") => ("user".to_string(), vault_name("user")),
                        s if s.starts_with("group:") => (s.to_string(), vault_name(it["scope_name"].as_str().unwrap_or(s))),
                        _ => (s.to_string(), vault_name(s)),
                    };
                    format!("in {vault}: {CLI} vault remove {}{} --scope {flag}", it["id"].as_str().unwrap_or(item), field.map(|f| format!(" {f}")).unwrap_or_default())
                })
                .collect();
            let there = match where_it.is_empty() {
                true => String::new(),
                false => format!("; it's {}", where_it.join(", and ")),
            };
            bail!("no item {item} in {}{there}", vault_name(scope));
        }
    };
    let title = it["title"].as_str().unwrap_or(item);
    match field {
        Some(field) => {
            post("/v1/vault", json!({ "scope": scope, "id": it["id"], "fields": { field: null } }))?;
            Ok(format!("Removed {title}'s {field}."))
        }
        None => {
            post("/v1/vault", json!({ "scope": scope, "id": it["id"], "deleted": true }))?;
            Ok(format!("Deleted {title}."))
        }
    }
}

/// Cloudflare's API with the token in CLOUDFLARE_API_TOKEN, or else the kernel's own (its .env.local's), for the account
/// `--account` names or the token's only one.
fn cloudflare(flags: &Map<String, Value>, settings: Option<&one::kernel::Settings>) -> Result<one::kernel::Cloudflare> {
    let token = std::env::var("CLOUDFLARE_API_TOKEN").ok().or_else(|| settings.and_then(|s| s.cloudflare_token.clone()));
    let token = token.context("give a Cloudflare API token that may edit Workers in CLOUDFLARE_API_TOKEN, or as the kernel's CLOUDFLARE_API_TOKEN in its .env.local")?;
    let api = "https://api.cloudflare.com/client/v4".to_string();
    let account = match flags.get("account").and_then(Value::as_str) {
        Some(account) => account.to_string(),
        None => one::kernel::Cloudflare { api: api.clone(), token: token.clone(), account: String::new() }.accounts()?,
    };
    Ok(one::kernel::Cloudflare { api, token, account })
}

/// Where a kernel is, and what it's set to, without its secrets.
fn kernel_said(s: &one::kernel::Settings) -> String {
    let mut out = format!("{}'s kernel\n  address   {}\n", s.name, s.address);
    if s.auth_url.starts_with("http://127.0.0.1") {
        out += &format!("  here on   127.0.0.1:{} (the address), :{} (auth)\n", s.ports[1], s.ports[0]);
    }
    if !s.resources.is_empty() {
        out += &format!("  nodes     {}\n", s.resources.join(", "));
    }
    out += &format!("  bucket    {}\n", s.bucket.as_ref().map(|b| format!("{} in {}", b["bucket"].as_str().unwrap_or("?"), b["endpoint"].as_str().unwrap_or("?"))).unwrap_or("none yet: kernel set bucket -".into()));
    if !s.android.is_empty() {
        out += &format!("  android   {} signed by {}\n", one::kernel::android_package(), s.android.join(", "));
    }
    out
}

/// Backups' status, for people.
fn backups_said(r: &Value) -> String {
    let mut out = String::new();
    let places = r["places"].as_array().cloned().unwrap_or_default();
    if places.is_empty() {
        return format!("Nothing is backed up yet: add a place with {CLI} backups place add.\n");
    }
    for p in &places {
        let when = |t: &Value| t.as_u64().map(|t| say_time(t)).unwrap_or_default();
        let last = match &p["last"] {
            Value::Null => "not backed up yet".to_string(),
            l => format!("backed up {} by {}", when(&l["time"]), l["device"].as_str().unwrap_or("")),
        };
        out.push_str(&format!("{} ({}): {last}\n", p["title"].as_str().unwrap_or(""), p["where"].as_str().unwrap_or("")));
        if let Some(problem) = p["problem"].as_object() {
            out.push_str(&format!("  {} {} (since {})\n", problem["text"].as_str().unwrap_or(""), problem["fix"].as_str().unwrap_or(""), when(&problem["since"])));
        }
    }
    out
}

/// Each step of trying a place, and whether it was added.
fn tried_said(r: &Value) -> String {
    let mut out = String::new();
    for s in r["tried"].as_array().into_iter().flatten() {
        match s["error"].as_str() {
            None => out.push_str(&format!("✓ {}\n", s["step"].as_str().unwrap_or(""))),
            Some(e) => out.push_str(&format!("✗ {}: {e}\n", s["step"].as_str().unwrap_or(""))),
        }
    }
    out
}

/// What a check restored, by name.
fn checked_said(r: &Value) -> String {
    let problems: Vec<&str> = r["problems"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if let Some(e) = r["error"].as_str().filter(|_| problems.is_empty()) {
        return format!("✗ {}: {e}\n", r["place"].as_str().unwrap_or(""));
    }
    if !problems.is_empty() {
        return format!("✗ {}: its newest backup didn't restore as it should:\n{}", r["place"].as_str().unwrap_or(""), problems.iter().map(|p| format!("  ✗ {p}\n")).collect::<String>());
    }
    let list = |k: &str| r[k].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
    let place = r["place"].as_str().unwrap_or("A disk");
    if r["scopes"].as_array().is_none_or(Vec::is_empty) {
        return match list("unchecked").as_str() {
            "" => format!("{place}: nothing backed up there yet\n"),
            unchecked => format!("{place}: not checked here, without their keys: {unchecked}\n"),
        };
    }
    let scopes: Vec<String> = r["scopes"].as_array().into_iter().flatten().map(|s| format!("{} ({} entries, {} blobs, from {})", s["name"].as_str().or(s["scope"].as_str()).unwrap_or(""), s["entries"], s["blobs"], say_time(s["time"].as_u64().unwrap_or(0)))).collect();
    let mut out = format!(
        "{}: {} came back, checked, from {:.1} MB there\n  ✓ every entry signed and in order, every blob as its hash says: {}\n",
        place,
        r["install"].as_str().unwrap_or(""),
        r["bytes"].as_f64().unwrap_or(0.0) / 1e6,
        scopes.join("; "),
    );
    if r["databases"] == true {
        out.push_str("  ✓ every database restores to its newest step, as the install has it\n");
    }
    out.push_str(&format!("  ✓ people: {}\n  ✓ apps: {}\n", list("people"), list("apps")));
    if !list("unchecked").is_empty() {
        out.push_str(&format!("  – not checked here, without their keys: {}\n", list("unchecked")));
    }
    out
}

/// Checks a backup on a disk with the recovery key alone, here, without a device: every install it holds.
fn check_folder(folder: &str) -> Result<Value> {
    let mut key = String::new();
    std::io::stdin().read_line(&mut key)?;
    let secret = one::entry::unhex(key.trim()).context("give the recovery key on stdin: 64 hex characters")?;
    let place = one::bucket::Place::Folder(folder.into());
    let mut out = Vec::new();
    for install in one::backups::installs(&place)? {
        let keys = one::backups::open_keys(&place, &install, &secret)?;
        let mut r = one::backups::check(&one::backups::Check { place: &place, install: &install, keys: &keys, root: None, live: None })?.0;
        r["place"] = json!(folder);
        out.push(r);
    }
    anyhow::ensure!(!out.is_empty(), "no backup in {folder}");
    Ok(json!(out))
}

/// A time, for people: today's as its hour, else its date.
fn say_time(ms: u64) -> String {
    let Ok(t) = jiff::Timestamp::from_millisecond(ms as i64) else { return String::new() };
    let tz = jiff::tz::TimeZone::system();
    let (t, today) = (t.to_zoned(tz.clone()), jiff::Timestamp::now().to_zoned(tz));
    match t.date() == today.date() {
        true => t.strftime("today at %H:%M").to_string(),
        false => t.strftime("%-d %b at %H:%M").to_string(),
    }
}

fn backups_tool(op: &str, input: &Map<String, Value>, agent: Option<&str>) -> Result<Value> {
    match op {
        "status" => get("/v1/backups"),
        "now" => post("/v1/backups/now", json!({ "agent": agent })).map(|_| json!("Backing up now; backups_status says how it went.")),
        "add_place" => {
            let r = post("/v1/backups/request", json!({ "endpoint": input.get("endpoint"), "title": input.get("title"), "agent": agent }))?;
            Ok(json!(format!("Asked. Give the person this link: {}\nThey type the bucket and its key into a sheet on their own device, and you never see it. Once they have, backups_status shows its first backup.", r["link"].as_str().unwrap_or(""))))
        }
        "remove_place" => {
            let place = input.get("place").and_then(Value::as_str).context("give place")?;
            let places = get("/v1/backups")?["places"].as_array().cloned().unwrap_or_default();
            let p = places.iter().find(|p| p["id"] == *place || p["title"] == *place).with_context(|| format!("no place {place}"))?;
            post("/v1/backups/place", json!({ "id": p["id"], "deleted": true, "agent": agent }))?;
            Ok(json!("Removed. What's there stays."))
        }
        "check" => post("/v1/backups/check", json!({})),
        other => anyhow::bail!("no tool backups_{other}"),
    }
}

/// The devices' tools, as this machine's person.
fn devices_tool(op: &str, input: &Map<String, Value>, client: &str) -> Result<Value> {
    let text = |k: &str| input.get(k).and_then(Value::as_str).with_context(|| format!("give {k}"));
    match op {
        "requests" => get("/v1/devices/requests"),
        "approve" => {
            let mut body = json!({ "code": text("code")?, "confirm": input.get("confirm") == Some(&json!(true)) });
            if let Some(at) = input.get("recovery_key").and_then(Value::as_str) {
                body["recovery"] = json!(recovery_key_from(at, client)?);
            }
            post("/v1/devices/approve", body).map(|r| json!(approved_said(&r)))
        }
        "remove" => {
            let what = match (input.get("device").and_then(Value::as_str), input.get("user").and_then(Value::as_str)) {
                (Some(device), None) => json!({ "key": device }),
                (None, Some(user)) => json!({ "user": user }),
                _ => anyhow::bail!("give device (one device) or user (a person, with all their devices)"),
            };
            anyhow::ensure!(input.get("confirm") == Some(&json!(true)), "devices_remove can't be undone, and the person hasn't agreed to it: ask them, and give confirm: true only if they say yes to this very device or person");
            post("/v1/remove", what).map(|r| json!(removed_said(&r)))
        }
        "keep" | "stop" => {
            let scope = input.get("scope").and_then(Value::as_str).unwrap_or("user");
            let days = if op == "stop" { json!(0) } else { input.get("days").cloned().unwrap_or(Value::Null) };
            post("/v1/grant", json!({ "app": text("app")?, "device": text("device")?, "scope": scope, "days": days })).map(|r| r["said"].clone())
        }
        "invite" => post("/v1/invite-link", json!({ "name": text("name")?, "admin": input.get("admin") == Some(&json!(true)) })).map(|r| json!(invited_said(text("name").unwrap_or(""), &r))),
        other => anyhow::bail!("no tool devices_{other}"),
    }
}

/// The install's recovery key, from the file at `at` on this machine, or stdin for `-` on the command line: never
/// typed into a chat, and never printed.
fn recovery_key_from(at: &str, client: &str) -> Result<String> {
    let looks_like_key = |t: &str| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit());
    anyhow::ensure!(!looks_like_key(at.trim()), "give the file the recovery key is in, never the key itself");
    let text = match at {
        "-" => {
            anyhow::ensure!(client == "cli", "- reads the recovery key from stdin on the command line: give a file's path instead");
            std::io::read_to_string(std::io::stdin())?
        }
        path => std::fs::read_to_string(path).with_context(|| format!("couldn't read {path}"))?,
    };
    let key = text.trim().to_ascii_lowercase();
    anyhow::ensure!(looks_like_key(&key), "that file doesn't hold a recovery key: 64 hex characters");
    Ok(key)
}

/// What approving said, in words: whose device it is, or what it was given.
fn approved_said(r: &Value) -> String {
    let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
    if let Some(service) = r["service"].as_str() {
        return match r.get("approved") {
            Some(_) => format!("{service} on {} is in: it answers your devices' calls.", s("device")),
            None => format!("{service} on {} wants to be yours. Approve it only if that's the code it shows.", s("device")),
        };
    }
    if r.get("sealed").is_none() && r["recovery"] == true {
        return format!("{} is {}'s device. Approve it with the recovery key only if they lost all their devices and say it's theirs.", s("device"), s("person"));
    }
    if r.get("sealed").is_none() {
        let new = if r["new"] == true { ", who you invited" } else { "" };
        return format!("{} is {}'s device{new}. Approve it only if they say it's theirs.", s("device"), s("person"));
    }
    let mut out = format!("{} is in.", s("device"));
    if r["recovery"] == true {
        out = format!("{} is in, let in with the recovery key: {} can open what they could before.", s("device"), s("person"));
    }
    if let Some(relay) = r["relay"].as_str() {
        out.push_str(&format!(" The network didn't take it yet: {relay}."));
    }
    for n in r["not_sealed"].as_array().into_iter().flatten() {
        out.push_str(&format!(" It wasn't given {}'s keys: {}.", n["scope"].as_str().unwrap_or(""), n["why"].as_str().unwrap_or("")));
    }
    out
}

/// What removing said, in words.
fn removed_said(r: &Value) -> String {
    let mut out = format!("Removed {}.", r["removed"].as_str().unwrap_or("it"));
    let left: Vec<&str> = r["not_renewed"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !left.is_empty() {
        out.push_str(&format!(" New keys for {} come from one of their own devices, or their admins'.", left.join(", ")));
    }
    out
}

/// An invite's link, in words.
fn invited_said(name: &str, r: &Value) -> String {
    let days = one::kernel::INVITE_DAYS;
    format!("Send {name} this link. It works once, for {days} days: opening it, they join. Then approve their device with its code.\n\n  {}", r["link"].as_str().unwrap_or(""))
}

fn vault_tool(op: &str, input: &Map<String, Value>, agent: Option<&str>) -> Result<Value> {
    let scope = input.get("scope").and_then(Value::as_str).unwrap_or("user");
    let text = |k: &str| input.get(k).and_then(Value::as_str).with_context(|| format!("give {k}"));
    match op {
        "list" => Ok(json!(get(&format!("/v1/vault?scope={scope}"))?.as_array().into_iter().flatten().map(item_said).collect::<Vec<_>>())),
        "set" => {
            let hosts = input.get("hosts").and_then(Value::as_array).map(|h| h.iter().filter_map(Value::as_str).map(str::to_string).collect());
            Ok(json!(vault_set(scope, text("item")?, text("field")?, text("value")?, input.get("secret").and_then(Value::as_bool), hosts, input.get("app").and_then(Value::as_str), agent)?))
        }
        "use" => Ok(json!(vault_use(scope, text("item")?, text("app")?, input.get("on") != Some(&json!(false)), agent)?)),
        "grant" => Ok(post("/v1/vault/grant", json!({ "scope": scope, "device": text("device")?, "on": input.get("on") != Some(&json!(false)) }))?["said"].clone()),
        "fetch" => post("/v1/vault/fetch", json!({ "url": text("url")?, "method": input.get("method"), "headers": input.get("headers"), "body": input.get("body"), "agent": agent })),
        "remove" => Ok(json!(vault_remove(scope, text("item")?, input.get("field").and_then(Value::as_str))?)),
        "request" => {
            // With no app, it's for agents: the person's own, with vault_fetch.
            let app = input.get("app").and_then(Value::as_str).unwrap_or("agents");
            let r = post("/v1/vault/request", json!({ "scope": scope, "app": app, "need": input.get("need"), "title": input.get("title"), "fields": input.get("fields"), "hosts": input.get("hosts"), "why": input.get("why"), "agent": agent }))?;
            Ok(json!(format!("Asked. Give the person this link: {}\nIt's also waiting in their Settings, under Requests. Tell them; check vault_status with id {} when they say they've done it.", r["link"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""))))
        }
        "status" => {
            let id = text("id")?;
            let r = get(&format!("/v1/vault/requests?id={id}"))?;
            let r = r.as_array().and_then(|a| a.first()).with_context(|| format!("no request {id}"))?.clone();
            Ok(json!({ "state": r["state"], "app": r["app"], "title": r["title"], "answered": r["answered"] }))
        }
        other => anyhow::bail!("no tool vault_{other}"),
    }
}

/// MCP's `deploy`, `{CLI} deploy` for an agent with no shell: the app's files as text, built where it's asked.
fn deploy_tool() -> Value {
    json!({
        "name": "deploy",
        "description": format!("Deploy an app from its files: builds it as {CLI} deploy builds a folder, npm packages and all, and makes the app if the name is new, owned by you. Or put back an earlier version. Answers with its tools and how to put it on your page."),
        "inputSchema": { "type": "object", "required": ["app"], "properties": {
            "app": { "type": "string", "description": "the app's name, lowercase words joined by -, like water-log; a new name makes a new app" },
            "files": { "type": "object", "additionalProperties": { "type": "string" }, "description": "every file of the app, each path → its text: { \"app.js\": \"export default { … }\" }, and package.json for npm packages" },
            "version": { "type": "string", "description": "instead of files: an earlier version to put back, from this tool's answer or apps_list" },
        } },
        "annotations": one::skills::hints(false, false),
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
    tools.extend(one::vault::tools());
    tools.extend(one::skills::groups_tools());
    // Backups are this machine's to run: a sign-in from elsewhere can't.
    if !signed_in() {
        tools.extend(one::backups::tools());
        tools.extend(one::skills::devices_tools());
    }
    for app in apps.as_array().into_iter().flatten() {
        let name = app["name"].as_str().unwrap_or("");
        // Operations starting with `_` are for the node, such as a service's `_screen`.
        for op in app["ops"].as_array().into_iter().flatten().filter(|op| !op["name"].as_str().unwrap_or("").starts_with('_')) {
            tools.push(json!({ "name": format!("{name}_{}", op["name"].as_str().unwrap_or("")), "description": op["description"], "inputSchema": ops::mcp_schema(app, op), "annotations": ops::mcp_annotations(op) }));
        }
    }
    Ok(tools)
}

/// A tool's answer, within the budget an agent reads (ops::BUDGET).
fn call_tool(params: &Value, agent: Option<&str>, mode: Mode) -> Value {
    let mut out = answer_tool(params, agent, mode);
    let name = params["name"].as_str().unwrap_or("");
    let chars: usize = out["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).map(str::len).sum();
    if chars > ops::BUDGET {
        let op = match name {
            "search" => Some(code::search_tool(mode)["inputSchema"].clone()).map(|input| json!({ "input": input })),
            _ => name.split_once('_').and_then(|(a, o)| ops::find(a, o)).map(|(_, o)| o),
        };
        ops::within_budget(&mut out["content"], op.as_ref());
    }
    out
}

fn answer_tool(params: &Value, agent: Option<&str>, mode: Mode) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let input = params["arguments"].as_object().cloned().unwrap_or_default();
    let failed = |text: String| json!({ "isError": true, "content": [{ "type": "text", "text": text }] });
    // This server's own tools, checked as apps' are, in dispatch.
    let own = [code::search_tool(mode), code::execute_tool(), deploy_tool()].into_iter().chain(mcp_apps::tools()).find(|t| t["name"] == name);
    if let Some(Err(e)) = own.map(|t| ops::check(&t["inputSchema"], &Value::Object(input.clone()))) {
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
            let body = match (input.get("files"), input.get("version")) {
                (Some(files), None) => json!({ "app": app, "files": files, "wait": true }),
                (None, Some(version)) => json!({ "app": app, "bundle": version, "wait": true }),
                _ => return failed("give files, the app's files, or version, an earlier one to put back".into()),
            };
            match post("/v1/deploy", body) {
                Ok(r) => {
                    let installed = get("/v1/apps").ok().and_then(|a| a.as_array()?.iter().find(|a| a["name"] == app).map(|a| a["installed"].as_array().is_some_and(|i| !i.is_empty())));
                    json!({ "content": [{ "type": "text", "text": deploy_said(app, &r, installed == Some(true)) }] })
                }
                Err(e) => failed(format!("{e:#}")),
            }
        }
        "execute" => match code::execute(input.get("code").and_then(Value::as_str).unwrap_or(""), agent, "mcp") {
            Ok(result) => json!({ "isError": result["ok"] != true, "content": [{ "type": "text", "text": code::shown(&result) }] }),
            Err(e) => failed(format!("{e:#}")),
        },
        _ => match dispatch(name, input, agent, "mcp", None) {
            // A script gets an empty list as it is; an agent reading gets it in words.
            Ok(ok) => match ["accounts", "devices"].into_iter().find_map(|op| Some((name.strip_suffix(&format!("_{op}"))?, op))).filter(|_| ok == json!([])).and_then(|(app, op)| Some((ops::find(app, op)?.0, op))).and_then(|(a, op)| ops::none_said(&a, op, &ok, false)) {
                Some(none) => json!({ "content": [{ "type": "text", "text": none }] }),
                None => json!({ "content": ops::mcp_content(&ok) }),
            },
            Err(e) => failed(format!("{e:#}")),
        },
    }
}

/// What to say for a tool nobody here lists: an app's operations when the app is right, else the nearest tools.
fn unknown_tool(name: &str) -> Result<String> {
    if let Some((app, op)) = name.split_once('_')
        && get("/v1/apps")?.as_array().into_iter().flatten().chain(&ops::node_apps()).any(|a| a["name"] == app)
    {
        return ops::unknown(app, Some(op), false);
    }
    let tools = tools().map_err(|e| anyhow::anyhow!("{}", e["message"].as_str().unwrap_or("")))?;
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    match ops::nearest(name, &names).as_slice() {
        [] => match name.split_once('_') {
            Some((app, op)) => ops::unknown(app, Some(op), false),
            None => Ok(format!("no tool {name}: search finds the tools for a task")),
        },
        near => Ok(format!("no tool {name}: did you mean {}?", near.join(" or "))),
    }
}

/// Runs one tool as the person, an app's operation or the Vault's, for MCP or a code mode script: what can't be undone
/// only with confirm: true. `spec` is the operation, when the caller has it already.
fn dispatch(name: &str, mut input: Map<String, Value>, agent: Option<&str>, client: &str, spec: Option<(Value, Value)>) -> Result<Value> {
    let Some((app, op)) = name.split_once('_') else { bail!("{}", unknown_tool(name)?) };
    let Some((a, spec)) = spec.or_else(|| ops::find(app, op)) else { bail!("{}", unknown_tool(name)?) };
    // Checked as MCP lists it, before anything runs: a wrong input says what's wrong, never runs as something else.
    ops::check(&ops::mcp_schema(&a, &spec), &Value::Object(input.clone()))?;
    let spec = Some(spec);
    let device = if ops::node_app(app) || spec.as_ref().is_some_and(ops::own_device) { None } else { input.remove("device") };
    let confirmed = input.remove("confirm") == Some(json!(true));
    if let Some(spec) = &spec {
        anyhow::ensure!(spec["confirm"] != true || confirmed, "{name} sends or can't be undone, and the person hasn't agreed to it: ask them, and give confirm: true only if they say yes to this very action");
    }
    if app == "vault" {
        return vault_tool(op, &input, agent);
    }
    if app == "backups" {
        return backups_tool(op, &input, agent);
    }
    if app == "devices" {
        // Its approve and remove read the person's yes themselves.
        if confirmed {
            input.insert("confirm".into(), json!(true));
        }
        return devices_tool(op, &input, client);
    }
    if name == "groups_list" {
        return Ok(json!(groups_said(&get("/v1/state")?)));
    }
    if name == "groups_handle" {
        let handle = input.get("handle").and_then(Value::as_str).context("give the new handle")?;
        post("/v1/handle", json!({ "handle": handle, "group": input.get("group") }))?;
        return Ok(json!(match input.get("group").and_then(Value::as_str) {
            Some(group) => format!("{group}'s handle is now {handle}."),
            None => format!("Your handle is now {handle}."),
        }));
    }
    if name == "widgets_show" {
        let out = mcp_apps::call(name, &Value::Object(input)).unwrap_or_default();
        anyhow::ensure!(out["isError"] != true, "{}", out["content"][0]["text"].as_str().unwrap_or(""));
        return Ok(out);
    }
    if let Some(spec) = spec {
        ops::mcp_files(&spec, &mut input)?;
    }
    Ok(post("/v1/call", json!({ "device": device, "app": app, "op": op, "input": input, "agent": agent, "client": client }))?["ok"].take())
}
