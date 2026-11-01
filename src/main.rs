use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use one::config::{CLI, NAME};
use one::skills::Mode;
use one::{api, node::Node};
use serde_json::{Map, Value, json};

mod code;
mod mcp_apps;
mod ops;
mod update;

fn help() -> String {
    format!(
        "{NAME}: your machines, and your team's, as one place for apps and agents.

Usage:
  {CLI} serve                          run this machine's server
  {CLI} status                         this server, its install, its users, groups and servers
  {CLI} create <name> --as <your name> create an install, with you as its first admin
  {CLI} pair [--new-user <name> [--admin] | --user <name>]
                                       a pairing code for another of your machines (or, for admins, a new user or someone else)
  {CLI} join <code>                    join an install with a pairing code
  {CLI} rename <name>                  rename this server
  {CLI} remove <server|client> | --user <user>
  {CLI} role <user> admin|member       (admins)
  {CLI} settings [<name> <value>|--default]   the install's settings (admins set them)
  {CLI} group create <name>            a group, with you as its admin
  {CLI} group invite <group> <user> [--admin]
  {CLI} group accept <group>
  {CLI} group remove <group> <user>    or leave it, giving yourself
  {CLI} acl <server> --owners a,b --users c,d
                                       who owns and uses a server: user or group names, or install
  {CLI} trust <server> | untrust <server>
  {CLI} open <user> <server>           read a user's data on a server you own, for an hour; they see it
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
  {CLI} vault fetch <url> [--method M] [--headers <json object>] [--body text]
                                       make a request with {{{{field}}}} filled in from the items the person lets
                                       agents use, a secret field only for its item's sites: you get the answer,
                                       never the secret
  {CLI} vault remove <item> [<field>]  delete an item, or one of its fields
  {CLI} vault request <app> (--need <name> | --title <t> --fields a,b [--secret-fields b] --hosts a,b) [--why text]
                                       ask the person for an item an app may use: a link to a sheet where they pick
                                       or make one, so you never see it; vault status <id> says how it went
  {CLI} vault requests                 the requests waiting for you; vault answer <id> [--item <id> | --decline]
                                       answers one, a new item's values on stdin (one line, or a JSON object)
  {CLI} backups                        how backups are going: each place, its last backup, what's failing
  {CLI} backups now                    back up now, without waiting for the hour
  {CLI} backups place add <folder> [--title t]
                                       back up to a folder on this server, like a USB disk's: tried before it's added
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
  {CLI} sidecars                       the server sidecars on each server: version, and who installed them
  {CLI} sidecars remove|add <name> [--server <name>]
                                       remove a sidecar from a server you own (here by default), or allow it there
                                       again; sidecars/install.sh <name> [server] installs one and allows it
  {CLI} schedule on|off <app> <name> [--scope user|global|<group>]
                                       switch an app's schedule on (it then runs as you) or off
  {CLI} <app>                          an app's operations
  {CLI} <app> <op> [words] [--input value]
                                       run an operation, e.g. {CLI} board post \"hi there\" --topic x; --help says
                                       what it takes. Required inputs may be words, lists repeat a flag, inputs not
                                       given are read as JSON from stdin, --file <input>=<path> gives a file and
                                       --out <path> saves what it gives back. --server <name> runs it there
  {CLI} try [<command>]                a new install, on this machine beside yours, with only what every new install
                                       starts with: to look around and test in. {CLI} try <command> runs a command in
                                       it ({CLI} try mcp for an agent), and {CLI} try stop deletes it
  {CLI} update                         test and build main's tip on this machine and the Macs, and run it everywhere
  {CLI} release <vX.Y.Z>               the same, signed off and published on GitHub for the installer
  {CLI} publish [binary]               sign and publish a build you made; every server on its platform updates
  {CLI} file put <file>                keep a file in the install, for apps and agents; prints its hash
  {CLI} file get <hash> <file>         get a kept file from this server or another
  {CLI} version                        this build
  {CLI} skills [<name> [<file>]]       the skills agents load: {NAME}'s own (how to connect, the rules, which MCP to
                                       add; also {CLI} guide, and /llms.txt), and each area's and app's
  {CLI} search <words>                 the operations for a task in every app: how to call each, and its app's guide
  {CLI} run <file>                     run a short script that calls operations, as tools.<app>.<op>({{ ... }}) (- reads
                                       it from stdin), and print what it returns; {CLI} search gives each call
  {CLI} mcp [--code]                   serve MCP over stdio, for agents: every tool, or with --code, search and execute

{NAME} holds one install. For another context, run that context's own app."
    )
}

fn addr() -> String {
    api::addr()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Linked as git-remote-<cli>, it's git's remote helper for <cli>://<repo>: git runs it with a remote and a URL.
    let invoked = std::env::args().next().map(|a| std::path::Path::new(&a).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()).unwrap_or_default();
    let result = match invoked == format!("git-remote-{CLI}") {
        true => one::git::remote(args.last().map(String::as_str).unwrap_or("")),
        false => run(args),
    };
    if let Err(err) = result {
        eprintln!("{CLI}: {err:#}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    if args.first().map(String::as_str) == Some("try") {
        return try_install(&args[1..]);
    }
    let (positional, flags) = parse(&args);
    if !matches!(positional.first().map(String::as_str), Some("serve" | "mcp")) {
        // Printing into `| head` ends quietly, as other commands do, instead of panicking.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    match positional.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["help"] => {
            println!("{}", help());
            println!("\n{}", connection());
        }
        ["serve"] => {
            usual_path();
            serve()?
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
                true => println!("{}", match &result["value"] { Value::String(s) => s.clone(), v => serde_json::to_string_pretty(v)? }),
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
            println!("{}", code::search(&code::catalog()?, &words.join(" "), limit, code::For::Cli));
        }
        ["status"] => status(&get("/v1/state")?),
        ["apps"] => {
            for app in get("/v1/apps")?.as_array().into_iter().flatten() {
                let from = app["from"].as_str().map(|f| format!(", a fork of {f}")).unwrap_or_default();
                let installed: Vec<&str> = app["installed"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let installed = if installed.is_empty() { String::new() } else { format!(", installed in {}", installed.join(", ")) };
                println!("{} ({}{from}{installed})", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or(""));
                ops::list(std::slice::from_ref(app)).iter().for_each(|l| println!("{l}"));
            }
        }
        ["create", name @ ..] if !name.is_empty() => {
            let user_name = flags.get("as").and_then(Value::as_str).context("say what to call you with --as <your name>")?;
            let r = post("/v1/create", json!({ "name": name.join(" "), "user_name": user_name }))?;
            println!("Created {}.\n\nIts recovery key, shown only this once; keep it in your admins' password manager:\n\n  {}\n\nAdd your other machines with: {CLI} pair", name.join(" "), r["recovery"].as_str().unwrap_or(""));
        }
        ["pair"] => {
            let new_user = flags.get("new-user").and_then(Value::as_str).map(|n| json!({ "name": n, "role": if flags.contains_key("admin") { "admin" } else { "member" } }));
            let r = post("/v1/pair", json!({ "new_user": new_user, "user": flags.get("user") }))?;
            println!("On the other machine, run:\n\n  {CLI} join {}\n\nThe pairing code works once, for a week.", r["code"].as_str().unwrap_or(""));
        }
        ["join", code] => {
            post("/v1/join", json!({ "code": code }))?;
            println!("Joined. Syncing now; {CLI} status shows progress.");
        }
        ["remove", key] => {
            post("/v1/remove", json!({ "key": key }))?;
            println!("Removed {key}.");
        }
        ["remove"] if flags.contains_key("user") => {
            post("/v1/remove", json!({ "user": flags["user"] }))?;
            println!("Removed them.");
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
        ["acl", server] => {
            let list = |key: &str| flags.get(key).and_then(Value::as_str).map(|s| s.split(',').map(str::trim).collect::<Vec<_>>());
            let owners = list("owners").context("give --owners")?;
            let users = list("users").unwrap_or_else(|| owners.clone());
            post("/v1/acl", json!({ "server": server, "owners": owners, "users": users }))?;
            println!("{server}'s ACL is set.");
        }
        ["trust", server] | ["untrust", server] => {
            let trust = positional[0] == "trust";
            post("/v1/trust", json!({ "server": server, "trust": trust }))?;
            println!("You {} {server}.", if trust { "trust" } else { "no longer trust" });
        }
        ["open", user, server] => {
            let r = post("/v1/open", json!({ "user": user, "server": server, "agent": agent_from_env() }))?;
            println!("{}", r["ok"].as_str().unwrap_or("Opened."));
        }
        ["deploy", app] if flags.contains_key("version") => {
            let version = flags["version"].as_str().context("give --version <hash>")?;
            post("/v1/deploy", json!({ "app": app, "bundle": version }))?;
            println!("Deployed {app} ({version}) again. Every server runs it now.");
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
        ["backups"] => print!("{}", backups_said(&get("/v1/backups")?)),
        ["backups", "now"] => {
            post("/v1/backups/now", json!({}))?;
            println!("Backing up now. {CLI} backups says how it went.");
        }
        ["backups", "place", "add"] | ["backups", "place", "add", _] => {
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
        }
        ["backups", "place", "remove", place] => {
            let places = get("/v1/backups")?["places"].as_array().cloned().unwrap_or_default();
            let p = places.iter().find(|p| p["id"] == *place || p["title"] == *place).with_context(|| format!("no place {place}; {CLI} backups lists them"))?;
            post("/v1/backups/place", json!({ "id": p["id"], "deleted": true }))?;
            println!("No longer backing up to {}. What's there stays.", p["title"].as_str().unwrap_or(""));
        }
        ["backups", "check"] => {
            let r = match flags.get("folder").and_then(Value::as_str) {
                Some(folder) => check_folder(folder)?,
                None => post("/v1/backups/check", json!({}))?,
            };
            for place in r.as_array().into_iter().flatten() {
                print!("{}", checked_said(place));
            }
        }
        ["vault"] | ["vault", "list"] => {
            let scope = flags.get("scope").and_then(Value::as_str).map(|s| format!("?scope={s}")).unwrap_or_default();
            for it in get(&format!("/v1/vault{scope}"))?.as_array().into_iter().flatten() {
                println!("{}", item_line(it));
            }
        }
        ["vault", "set", item, field] => {
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
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            println!("{}", vault_use(scope, item, app, !flags.contains_key("off"), agent_from_env().as_deref())?);
        }
        ["vault", "fetch", url] => {
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
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            println!("{}", vault_remove(scope, item, positional.get(3).map(String::as_str))?);
        }
        ["vault", "request", app] => {
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let list = |k: &str| flags.get(k).and_then(Value::as_str).map(|v| v.split(',').map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
            let secret = list("secret-fields");
            let fields: Map<String, Value> = list("fields").into_iter().map(|f| (f.clone(), json!({ "secret": secret.contains(&f) }))).collect();
            let body = json!({ "scope": scope, "app": app, "need": flags.get("need"), "title": flags.get("title"), "fields": fields, "hosts": list("hosts"), "why": flags.get("why"), "agent": agent_from_env() });
            let r = post("/v1/vault/request", body)?;
            println!("Open this to answer it: {}\n{CLI} vault status {} says how it went.", r["link"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""));
        }
        ["vault", "status", id] => {
            let r = get(&format!("/v1/vault/requests?id={id}"))?;
            let r = r.as_array().and_then(|a| a.first()).with_context(|| format!("no request {id}"))?.clone();
            println!("{} for {} in {}: {}", r["title"].as_str().unwrap_or(""), r["app"].as_str().unwrap_or(""), r["scope"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""));
        }
        ["vault", "requests"] => {
            for r in get("/v1/vault/requests")?.as_array().into_iter().flatten() {
                println!("{}  {} for {} in {}  {}  {}", r["id"].as_str().unwrap_or(""), r["title"].as_str().unwrap_or(""), r["app"].as_str().unwrap_or(""), r["scope"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""), r["why"].as_str().unwrap_or(""));
            }
        }
        ["vault", "answer", id] => {
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
        ["secret", "request", app, name] => {
            let hosts: Vec<&str> = flags.get("hosts").and_then(Value::as_str).context("give the hosts it's sent to: --hosts api.example.com")?.split(',').map(str::trim).collect();
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let r = post("/v1/secrets/request", json!({ "scope": scope, "app": app, "name": name, "hosts": hosts, "why": flags.get("why"), "agent": agent_from_env() }))?;
            println!("Open this to set it: {}\nOr: {CLI} secret answer {} < file\n{CLI} secret status {} says how it went.", r["link"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""));
        }
        ["secret", "status", id] => {
            let r = get(&format!("/v1/secrets/requests?id={id}"))?;
            let r = r.as_array().and_then(|a| a.first()).with_context(|| format!("no request {id}"))?.clone();
            println!("{}/{} in {}: {}", r["app"].as_str().unwrap_or(""), r["name"].as_str().unwrap_or(""), r["scope"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""));
        }
        ["secret", "requests"] => {
            for r in get("/v1/secrets/requests")?.as_array().into_iter().flatten() {
                println!("{}  {}/{} in {}  {}  {}", r["id"].as_str().unwrap_or(""), r["app"].as_str().unwrap_or(""), r["name"].as_str().unwrap_or(""), r["scope"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""), r["why"].as_str().unwrap_or(""));
            }
        }
        ["secret", "answer", id] => {
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let value = match flags.contains_key("decline") {
                true => Value::Null,
                false => {
                    let mut v = String::new();
                    std::io::stdin().read_line(&mut v)?;
                    let v = v.trim_end_matches(['\n', '\r']).to_string();
                    anyhow::ensure!(!v.is_empty(), "give the secret on stdin, or --decline");
                    json!(v)
                }
            };
            post("/v1/secrets/answer", json!({ "scope": scope, "id": id, "value": value }))?;
            println!("{}", if value.is_null() { "Declined." } else { "Set." });
        }
        ["secret", "set", app, name] => {
            let hosts: Vec<&str> = flags.get("hosts").and_then(Value::as_str).context("give the hosts it's sent to: --hosts api.example.com")?.split(',').map(str::trim).collect();
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
            anyhow::ensure!(!value.is_empty(), "give the secret on stdin");
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            post("/v1/secrets", json!({ "scope": scope, "app": app, "name": name, "hosts": hosts, "value": value }))?;
            println!("{app}'s secret {name} is set in {scope}; it's sent only to {}.", hosts.join(", "));
        }
        ["secret", "list"] => {
            let q = flags.get("scope").and_then(Value::as_str).map(|s| format!("?scope={s}")).unwrap_or_default();
            for s in get(&format!("/v1/secrets{q}"))?.as_array().into_iter().flatten() {
                let hosts: Vec<&str> = s["hosts"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let kept = if s["given"] == true { "" } else { "  (kept by the app)" };
                println!("{}  {}/{}  → {}{kept}", s["scope"].as_str().unwrap_or(""), s["app"].as_str().unwrap_or(""), s["name"].as_str().unwrap_or(""), hosts.join(", "));
            }
        }
        ["secret", "remove", app, name] => {
            let scope = flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            post("/v1/secrets", json!({ "scope": scope, "app": app, "name": name, "value": null }))?;
            println!("{app}'s secret {name} is gone from {scope}.");
        }
        ["app", how @ ("delete" | "restore" | "erase"), app] => {
            let past = match *how { "delete" => "deleted", "restore" => "restored", _ => "erased" };
            post("/v1/retire", json!({ "app": app, "how": past }))?;
            println!("{app} is {past}{}.", if *how == "delete" { ": it's in the App Store's Recently Deleted until you restore or erase it" } else { "" });
        }
        // `drivers`: what sidecars were called first.
        ["sidecars" | "drivers"] => {
            let state = get("/v1/state")?;
            for s in state["install"]["servers"].as_array().into_iter().flatten() {
                let installed = s["installed_sidecars"].as_array().cloned().unwrap_or_default();
                let removed = s["removed_sidecars"].as_array().cloned().unwrap_or_default();
                if installed.is_empty() && removed.is_empty() {
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
                for d in &removed {
                    println!("  {}  removed by {} {}", d["name"].as_str().unwrap_or(""), d["by"].as_str().unwrap_or("an owner"), ago(d["time"].as_u64().unwrap_or(0)));
                }
            }
        }
        ["sidecars" | "drivers", how @ ("remove" | "add"), name] => {
            let server = flags.get("server").and_then(Value::as_str);
            post("/v1/sidecar", json!({ "name": name, "server": server, "installed": *how == "add" }))?;
            let on = server.map(|s| format!("on {s}")).unwrap_or_else(|| "here".into());
            match *how {
                "remove" => println!("{name} is removed {on}: it isn't served there until an owner adds it again ({CLI} sidecars add {name})."),
                _ => println!("{name} is installed {on}: it serves there while it runs."),
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
            let bytes = std::fs::read(file).with_context(|| format!("read {file}"))?;
            println!("{}", request(agent().post(format!("http://{}/v1/blobs", addr())).send(&bytes[..]))?["blob"].as_str().unwrap_or(""));
        }
        ["file" | "blob", "get", hash, file] => {
            let mut resp = agent().get(format!("http://{}/v1/blobs/{hash}", addr())).call().with_context(|| format!("{NAME} isn't running here: start it with {CLI} serve"))?;
            if !resp.status().is_success() {
                bail!("{}", resp.body_mut().read_json::<Value>()?["error"].as_str().unwrap_or("failed"));
            }
            std::fs::write(file, resp.body_mut().with_config().limit(1 << 30).read_to_vec()?)?;
        }
        ["version"] if flags.contains_key("json") => println!("{}", json!({ "commit": one::config::BUILD, "target": one::config::TARGET, "committed": one::config::COMMITTED })),
        ["version"] => println!("{} {} ({})", NAME, one::config::BUILD, one::config::TARGET),
        ["rename", name @ ..] if !name.is_empty() => {
            post("/v1/name", json!({ "name": name.join(" ") }))?;
            println!("This server is now {}.", name.join(" "));
        }
        [app] if !app.starts_with('-') => {
            let apps = get("/v1/apps")?;
            let found = apps.as_array().into_iter().flatten().find(|a| a["name"] == *app).with_context(|| format!("unknown command {app}; {CLI} help lists them, {CLI} apps every app's"))?;
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
        _ => bail!("unknown command; {CLI} help lists them"),
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
    println!("Published {} for {}. Every server on that platform updates itself now.", r["commit"].as_str().unwrap_or(""), r["target"].as_str().unwrap_or(""));
    Ok(())
}

/// Deploys the app in `dir`: builds it with its source into a bundle, makes the app if the install hasn't one by that
/// name, and deploys it. Every server runs it now.
fn deploy(dir: &std::path::Path, flags: &Map<String, Value>) -> Result<()> {
    let name = match flags.get("name").and_then(Value::as_str) {
        Some(name) => name.to_string(),
        None => dir.canonicalize()?.file_name().context("which app?")?.to_string_lossy().to_string(),
    };
    let bundle = one::deploy::build(dir)?;
    let blob = request(agent().post(format!("http://{}/v1/blobs", addr())).send(&bundle[..]))?["blob"].as_str().unwrap_or("").to_string();
    let apps = get("/v1/apps")?;
    // A sidecar of the same name is listed too, without an owner: the app is still to be made.
    if !apps.as_array().into_iter().flatten().any(|a| a["name"] == name.as_str() && !a["owner"].is_null()) {
        let owner = match flags.get("owner").and_then(Value::as_str) {
            Some(owner) => owner.to_string(),
            None => format!("user:{}", get("/v1/state")?["install"]["user"].as_str().unwrap_or("")),
        };
        post("/v1/app", json!({ "name": name, "owner": owner }))?;
    }
    post("/v1/deploy", json!({ "app": name, "bundle": blob }))?;
    println!("Deployed {name} ({}). Every server runs it now.", &blob[..12]);
    Ok(())
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

/// Whether a server runs here, and which install it's in, for the bare command.
fn connection() -> String {
    let Ok(state) = get("/v1/state") else { return format!("{NAME} isn't running here: start it with {CLI} serve.") };
    let server = state["server"]["name"].as_str().unwrap_or("this server");
    let i = &state["install"];
    if i.is_null() {
        return format!("{server} is in no install yet: {CLI} create <name> --as <your name>, or {CLI} join <code>.");
    }
    let install = i["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("an install (still joining)");
    let me = state["user"]["name"].as_str().or(state["me"]["name"].as_str()).unwrap_or("");
    let ready = if state["server"]["ready"] == true { "" } else { ", catching up" };
    format!("Connected to {install} as {me}, on {server}{ready}. {CLI} status shows its servers; {CLI} apps what you can run.")
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

fn request(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
    let mut resp = resp.with_context(|| format!("{NAME} isn't running here: start it with {CLI} serve"))?;
    let body: Value = resp.body_mut().read_json()?;
    if let Some(err) = body.get("error") {
        bail!("{}", err.as_str().unwrap_or("failed"));
    }
    Ok(body)
}

fn get(path: &str) -> Result<Value> {
    request(agent().get(format!("http://{}{path}", addr())).call())
}

fn post(path: &str, body: Value) -> Result<Value> {
    request(agent().post(format!("http://{}{path}", addr())).send_json(&body))
}

fn status(state: &Value) {
    let server = &state["server"];
    println!("{} ({}){}", server["name"].as_str().unwrap_or(""), &server["key"].as_str().unwrap_or("")[..8], if server["ready"] == true { "" } else { ", not ready yet" });
    let i = &state["install"];
    if i.is_null() {
        println!("\nIn no install yet. Create one with {CLI} create <name> --as <your name>, or join with a pairing code.");
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
            let members: Vec<String> = g["members"].as_array().into_iter().flatten().map(|m| format!("{}{}", name(m["user"].as_str().unwrap_or("")), if m["role"] == "admin" { "*" } else { "" })).collect();
            println!("  {}  {}", g["name"].as_str().unwrap_or(""), members.join(", "));
        }
    }
    println!("\nServers");
    for s in i["servers"].as_array().into_iter().flatten() {
        let dot = if s["online"] == true { "●" } else { "○" };
        let here = if s["here"] == true { " (this one)" } else { "" };
        let owners: Vec<String> = s["owners"].as_array().into_iter().flatten().filter_map(|o| o.as_str()).map(|o| o.strip_prefix("user:").map(name).unwrap_or(o.into())).collect();
        let privileged = if s["privileged"] == true { ", privileged" } else { "" };
        let serves = if s["serves_me"] == true { "" } else { ", not yours" };
        let build = s["build"].as_str().map(|b| format!("  {b}")).unwrap_or_default();
        println!("  {dot} {}{here}  owned by {}{privileged}{serves}{build}", s["name"].as_str().unwrap_or(""), owners.join(", "));
    }
    let clients = i["clients"].as_array().cloned().unwrap_or_default();
    if !clients.is_empty() {
        println!("\nPhones");
        for c in &clients {
            // A phone keeps no log and holds no sessions: say when it last asked this server, not that it's offline.
            let asked = c["asked"].as_u64().map(|t| format!(", asked here {}", ago(t))).unwrap_or_default();
            println!("  ▯ {}  {}{asked}", c["name"].as_str().unwrap_or(""), name(c["user"].as_str().unwrap_or("")));
        }
    }
    for o in i["opens"].as_array().into_iter().flatten() {
        println!("\n{} opened your data on a server {}.", o["by_name"].as_str().unwrap_or("someone"), ago(o["time"].as_u64().unwrap_or(0)));
    }
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
/// sidecars it starts need Homebrew's and the user's own tools.
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

/// `try`: an install of its own, made as `create` makes every new one, by a node beside this machine's own with its own
/// home and port, so it touches nothing of the person's (src/agents.rs leaves the harnesses alone for a node given a
/// home). A command after `try` runs in it; it starts one first if none is running.
fn try_install(rest: &[String]) -> Result<()> {
    let dir = PathBuf::from(format!("{}-try", one::home().display()));
    let addr_var = format!("{}_ADDR", CLI.to_uppercase());
    let running = std::fs::read_to_string(dir.join("addr")).ok().map(|a| a.trim().to_string()).filter(|a| agent().get(format!("http://{a}/v1/state")).call().is_ok());
    match rest.first().map(String::as_str) {
        Some("stop") => {
            stop_try(&dir)?;
            println!("Stopped and deleted the try install. {CLI} try starts a new one.");
            return Ok(());
        }
        Some(cmd @ ("serve" | "update" | "release" | "publish" | "try")) => bail!("{CLI} {cmd} isn't for the try install"),
        _ => {}
    }
    let addr = match running {
        Some(addr) => addr,
        None => {
            eprintln!("Starting a new try install…");
            // Called what the person is called in their own install, if this machine is in one.
            let me = get("/v1/state").ok().and_then(|s| s["user"]["name"].as_str().or(s["me"]["name"].as_str()).map(String::from));
            let me = me.or_else(|| std::env::var("USER").ok()).unwrap_or_else(|| "You".into());
            let addr = start_try(&dir)?;
            // Before any thread starts: the CLI has none.
            unsafe { std::env::set_var(&addr_var, &addr) };
            post("/v1/create", json!({ "name": "Try", "user_name": me }))?;
            addr
        }
    };
    unsafe { std::env::set_var(&addr_var, &addr) };
    if !rest.is_empty() {
        return run(rest.to_vec());
    }
    let state = get("/v1/state")?;
    let me = state["user"]["name"].as_str().or(state["me"]["name"].as_str()).unwrap_or("you");
    let store = state["install"]["settings"]["store"].as_str().unwrap_or("apps");
    println!(
        "The try install is running beside yours, on {addr}: a new install, Try, with only what every new install
starts with, and {me} as its admin.

  {CLI} try <command>   any command, in it: {CLI} try status, {CLI} try apps, {CLI} try board post hello
  {CLI} try mcp         its MCP server, for an agent: claude mcp add {CLI}-try -- {CLI} try mcp
  {CLI} try stop        stop and delete it; {CLI} try then starts a new one

Its App Store, in a browser: http://{addr}/v1/screens/{store}/main"
    );
    Ok(())
}

/// Starts the try install's node, in `dir` made empty but for what it downloaded or built before, and gives its address
/// once it answers.
fn start_try(dir: &std::path::Path) -> Result<String> {
    use std::os::unix::process::CommandExt;
    stop_try(dir)?;
    std::fs::create_dir_all(dir)?;
    let free = |port| std::net::TcpListener::bind(("127.0.0.1", port)).ok().and_then(|l| l.local_addr().ok());
    let addr = free(one::config::PORT + 100).or_else(|| free(0)).context("no free port for the try install")?.to_string();
    let log = std::fs::File::create(dir.join("node.log"))?;
    let child = std::process::Command::new(std::env::current_exe()?)
        .arg("serve")
        .env(format!("{}_HOME", CLI.to_uppercase()), dir)
        .env(format!("{}_ADDR", CLI.to_uppercase()), &addr)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        // Its own group, so it outlives this command, and stopping it stops what it started.
        .process_group(0)
        .spawn()?;
    std::fs::write(dir.join("pid"), child.id().to_string())?;
    std::fs::write(dir.join("addr"), &addr)?;
    let start = std::time::Instant::now();
    while agent().get(format!("http://{addr}/v1/state")).call().is_err() {
        if start.elapsed() > std::time::Duration::from_secs(60) {
            bail!("the try install's node didn't start: {}", dir.join("node.log").display());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Ok(addr)
}

/// Stops the try install's node, if it runs, and deletes all of it but what it downloaded or built.
fn stop_try(dir: &std::path::Path) -> Result<()> {
    if let Some(pid) = std::fs::read_to_string(dir.join("pid")).ok().and_then(|p| p.trim().parse::<i32>().ok()) {
        let start = std::time::Instant::now();
        unsafe { libc::kill(-pid, libc::SIGTERM) };
        while unsafe { libc::kill(pid, 0) } == 0 && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name() != "cache" {
            let path = entry.path();
            if path.is_dir() { std::fs::remove_dir_all(&path)? } else { std::fs::remove_file(&path)? }
        }
    }
    Ok(())
}

#[tokio::main]
async fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn,iroh=error,noq_proto=error".into()))
        .init();
    let dir = one::home();
    let (node, router) = Node::start(&dir, one::sidecars::built_in()).await?;
    let listener = tokio::net::TcpListener::bind(addr()).await.with_context(|| format!("listen on {}", addr()))?;
    tracing::info!("{NAME} node {} serving {} from {}", &one::entry::hex(&node.id())[..8], addr(), dir.display());
    tokio::spawn(api::serve_guide(node.clone()));
    let http = axum::serve(listener, api::router(node));
    tokio::select! {
        r = http => r?,
        _ = tokio::signal::ctrl_c() => {}
    }
    router.shutdown().await.ok();
    Ok(())
}

/// MCP over stdio: every app operation is a tool, with `search` to find them; or in code mode (`--code`), `search` and
/// `execute`, which runs scripts with them, and the few tools every agent needs from the start.
fn mcp(mode: Mode) -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    // The agent this serves, credited on its calls: the client's own name for itself, else what its environment says.
    let mut agent = agent_from_env();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue };
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => Ok({
                if let Some(name) = msg["params"]["clientInfo"]["name"].as_str().filter(|n| !n.trim().is_empty()) {
                    agent = Some(agent_name(name));
                }
                json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {}, "resources": {}, "extensions": { "io.modelcontextprotocol/skills": {} } },
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
            "tools/call" => Ok(mcp_apps::call(msg["params"]["name"].as_str().unwrap_or(""), &msg["params"]["arguments"])
                .unwrap_or_else(|| call_tool(&msg["params"], agent.as_deref(), mode))),
            // Apps' widgets and screens, as MCP Apps (src/mcp_apps.rs).
            "resources/list" => Ok(json!({ "resources": mcp_apps::resources() })),
            "resources/read" => match msg["params"]["uri"].as_str().unwrap_or("") {
                uri if uri.starts_with("skill://") => skill_resource(uri),
                uri => mcp_apps::read(uri),
            },
            // Skills over MCP (SEP-2640): the same skills the agents sidecar writes into harnesses' folders.
            "skills/list" => Ok(json!({ "resultType": "complete", "skills": skills().iter().map(skill_entry).collect::<Vec<_>>(), "ttlMs": 60_000, "cacheScope": "session" })),
            "skills/get" => {
                let uri = msg["params"]["uri"].as_str().unwrap_or("");
                skills().iter().find(|s| format!("skill://{}/SKILL.md", s.name) == uri).map(|s| json!({ "skill": skill_entry(s) })).ok_or_else(|| json!({ "code": -32602, "message": format!("no skill {uri}") }))
            }
            other => Err(json!({ "code": -32601, "message": format!("unknown method {other}") })),
        };
        let reply = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
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
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": text }] }))
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

/// The server's instructions (src/guide.rs): the rules first, short enough that every harness keeps all of them.
fn instructions(mode: Mode) -> String {
    let apps = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
    one::skills::instructions(mode, &apps)
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
        "{}  {}{}  [{}]  {}{}  used by: {}",
        it["id"].as_str().unwrap_or(""),
        it["title"].as_str().unwrap_or(""),
        it["type"].as_str().filter(|t| !t.is_empty()).map(|t| format!(" ({t})")).unwrap_or_default(),
        it["scope_name"].as_str().unwrap_or(""),
        fields.join(", "),
        Some(list(&it["hosts"])).filter(|h| !h.is_empty()).map(|h| format!("  → {h}")).unwrap_or_default(),
        list(&it["apps"]),
    )
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
    Ok(match made {
        true => format!("Made {item} ({id}) in {scope}, with {field}."),
        false => format!("Set {item}'s {field}."),
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

/// Deletes `item` in `scope`, or with `field`, one of its fields.
fn vault_remove(scope: &str, item: &str, field: Option<&str>) -> Result<String> {
    let it = find_item(scope, item)?.with_context(|| format!("no item {item} in {scope}"))?;
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
            l => format!("backed up {} by {}", when(&l["time"]), l["server"].as_str().unwrap_or("")),
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
    out.push_str(match r["id"].is_string() {
        true => "Added: the first backup starts now, then every hour that something changes.\n",
        false => "Not added.\n",
    });
    out
}

/// What a check restored, by name.
fn checked_said(r: &Value) -> String {
    if let Some(e) = r["error"].as_str() {
        return format!("✗ {}: {e}\n", r["place"].as_str().unwrap_or(""));
    }
    let list = |k: &str| r[k].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
    let scopes: Vec<String> = r["scopes"].as_array().into_iter().flatten().map(|s| format!("{} ({} entries, {} blobs, from {})", s["scope"].as_str().unwrap_or(""), s["entries"], s["blobs"], say_time(s["time"].as_u64().unwrap_or(0)))).collect();
    format!(
        "{}: {} came back, checked, from {:.1} MB there\n  ✓ every entry signed and in order, every blob as its hash says: {}\n  ✓ people: {}\n  ✓ apps: {}\n  ✓ Vault: {}\n",
        r["place"].as_str().unwrap_or("A disk"),
        r["install"].as_str().unwrap_or(""),
        r["bytes"].as_f64().unwrap_or(0.0) / 1e6,
        scopes.join("; "),
        list("people"),
        list("apps"),
        list("vault"),
    )
}

/// Checks a backup on a disk with the recovery key alone, here, without a server: every install it holds.
fn check_folder(folder: &str) -> Result<Value> {
    let mut key = String::new();
    std::io::stdin().read_line(&mut key)?;
    let secret = one::entry::unhex(key.trim()).context("give the recovery key on stdin: 64 hex characters")?;
    let place = one::backups::Place::Folder(folder.into());
    let mut out = Vec::new();
    for install in one::backups::installs(&place)? {
        let keys = one::backups::open_keys(&place, &install, &secret)?;
        let mut r = one::backups::check(&place, &install, &keys, None)?;
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

fn vault_tool(op: &str, input: &Map<String, Value>, agent: Option<&str>) -> Result<Value> {
    let scope = input.get("scope").and_then(Value::as_str).unwrap_or("user");
    let text = |k: &str| input.get(k).and_then(Value::as_str).with_context(|| format!("give {k}"));
    match op {
        "list" => get(&format!("/v1/vault?scope={scope}")),
        "set" => {
            let hosts = input.get("hosts").and_then(Value::as_array).map(|h| h.iter().filter_map(Value::as_str).map(str::to_string).collect());
            Ok(json!(vault_set(scope, text("item")?, text("field")?, text("value")?, input.get("secret").and_then(Value::as_bool), hosts, input.get("app").and_then(Value::as_str), agent)?))
        }
        "use" => Ok(json!(vault_use(scope, text("item")?, text("app")?, input.get("on") != Some(&json!(false)), agent)?)),
        "fetch" => post("/v1/vault/fetch", json!({ "url": text("url")?, "method": input.get("method"), "headers": input.get("headers"), "body": input.get("body"), "agent": agent })),
        "remove" => Ok(json!(vault_remove(scope, text("item")?, input.get("field").and_then(Value::as_str))?)),
        "request" => {
            let r = post("/v1/vault/request", json!({ "scope": scope, "app": input.get("app"), "need": input.get("need"), "title": input.get("title"), "fields": input.get("fields"), "hosts": input.get("hosts"), "why": input.get("why"), "agent": agent }))?;
            Ok(json!(format!("Asked. Give the person this link: {}\nIt's also waiting in their Settings, under Requests. Poll vault_status with id {} until it's set.", r["link"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""))))
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

/// The tools' earlier names, `secrets_*`, still answered (not listed) for agents that learned them.
fn secret_tool(op: &str, input: &Map<String, Value>, agent: Option<&str>) -> Result<Value> {
    let scope = input.get("scope").and_then(Value::as_str).unwrap_or("user");
    match op {
        "request" => {
            let r = post("/v1/secrets/request", json!({ "scope": scope, "app": input.get("app"), "name": input.get("name"), "hosts": input.get("hosts"), "why": input.get("why"), "agent": agent }))?;
            Ok(json!(format!("Asked. Give the person this link to set it: {}\nIt's also waiting in their Settings, under Requests. Poll secrets_status with id {} until it's set.", r["link"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""))))
        }
        "status" => {
            let id = input.get("id").and_then(Value::as_str).context("give the request's id")?;
            let r = get(&format!("/v1/secrets/requests?id={id}"))?;
            let r = r.as_array().and_then(|a| a.first()).with_context(|| format!("no request {id}"))?.clone();
            Ok(json!({ "state": r["state"], "app": r["app"], "name": r["name"], "answered": r["answered"] }))
        }
        "set" => post("/v1/secrets", json!({ "scope": scope, "app": input.get("app"), "name": input.get("name"), "hosts": input.get("hosts"), "value": input.get("value") })).map(|_| json!("Set.")),
        "remove" => post("/v1/secrets", json!({ "scope": scope, "app": input.get("app"), "name": input.get("name"), "value": null })).map(|_| json!("Removed.")),
        _ => get(&format!("/v1/secrets{}", input.get("scope").and_then(Value::as_str).map(|s| format!("?scope={s}")).unwrap_or_default())),
    }
}

fn tools() -> Result<Vec<Value>, Value> {
    let apps = get("/v1/apps").map_err(|e| json!({ "code": -32000, "message": e.to_string() }))?;
    let mut tools = one::vault::tools();
    tools.extend(one::backups::tools());
    for app in apps.as_array().into_iter().flatten() {
        let name = app["name"].as_str().unwrap_or("");
        // Operations starting with `_` are for the node, such as a sidecar's `_screen`.
        for op in app["ops"].as_array().into_iter().flatten().filter(|op| !op["name"].as_str().unwrap_or("").starts_with('_')) {
            let mut tool = json!({ "name": format!("{name}_{}", op["name"].as_str().unwrap_or("")), "description": op["description"], "inputSchema": ops::mcp_schema(app, op) });
            let hints = ops::mcp_annotations(op);
            if hints.as_object().is_some_and(|h| !h.is_empty()) {
                tool["annotations"] = hints;
            }
            tools.push(tool);
        }
    }
    Ok(tools)
}

fn call_tool(params: &Value, agent: Option<&str>, mode: Mode) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let input = params["arguments"].as_object().cloned().unwrap_or_default();
    let failed = |text: String| json!({ "isError": true, "content": [{ "type": "text", "text": text }] });
    match name {
        "search" => {
            let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
            match code::catalog() {
                Ok(apps) => json!({ "content": [{ "type": "text", "text": code::search(&apps, input.get("query").and_then(Value::as_str).unwrap_or(""), limit, code::For::Mcp(mode)) }] }),
                Err(e) => failed(format!("{e:#}")),
            }
        }
        "execute" => match code::execute(input.get("code").and_then(Value::as_str).unwrap_or(""), agent, "mcp") {
            Ok(result) => json!({ "isError": result["ok"] != true, "content": [{ "type": "text", "text": code::shown(&result) }] }),
            Err(e) => failed(format!("{e:#}")),
        },
        _ => match dispatch(name, input, agent, "mcp", None) {
            Ok(ok) => json!({ "content": ops::mcp_content(&ok) }),
            Err(e) => failed(format!("{e:#}")),
        },
    }
}

/// Runs one tool as the person, an app's operation or the Vault's, for MCP or a code mode script: what can't be undone
/// only with confirm: true. `spec` is the operation, when the caller has it already.
fn dispatch(name: &str, mut input: Map<String, Value>, agent: Option<&str>, client: &str, spec: Option<Value>) -> Result<Value> {
    let (app, op) = name.split_once('_').with_context(|| format!("no tool {name}"))?;
    if app == "vault" {
        return vault_tool(op, &input, agent);
    }
    if app == "secrets" {
        return secret_tool(op, &input, agent);
    }
    if app == "backups" {
        return backups_tool(op, &input, agent);
    }
    let server = input.remove("server").or(input.remove("device"));
    let confirmed = input.remove("confirm") == Some(json!(true));
    if let Some(spec) = spec.or_else(|| ops::find(app, op).map(|(_, spec)| spec)) {
        anyhow::ensure!(spec["confirm"] != true || confirmed, "{name} can't be undone: ask the person first, then call it again with confirm: true");
        ops::mcp_files(&spec, &mut input)?;
    }
    Ok(post("/v1/call", json!({ "server": server, "app": app, "op": op, "input": input, "agent": agent, "client": client }))?["ok"].take())
}
