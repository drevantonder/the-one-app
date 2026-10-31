use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use one::brand::{CLI, NAME};
use one::{api, node::Node};
use serde_json::{Map, Value, json};

mod mcp_apps;
mod ops;
mod update;

fn help() -> String {
    format!(
        "{NAME}: your machines, and your team's, as one place for apps and agents.

Usage:
  {CLI} serve                          run this machine's node
  {CLI} status                         this server, its install, its users, groups and servers
  {CLI} create <name> --as <your name> create an install, with you as its first admin
  {CLI} pair [--new-user <name> [--admin]]
                                       a pairing code for another of your machines (or, for admins, a new user)
  {CLI} join <code>                    join an install with a pairing code
  {CLI} rename <name>                  rename this server
  {CLI} remove <server|client> | --user <user>
  {CLI} role <user> admin|member       (admins)
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
  {CLI} deploy <app> --bundle <hash>   deploy an earlier bundle again: a rollback
  {CLI} deploys <app>                  an app's deploys, newest last
  {CLI} source <app> [path] [--bundle <hash>]
                                       an app's code as deployed: its files, or one of them
  {CLI} install <app> [--scope user|global|<group>]
                                       install an app, and the apps it uses, for you, a group you admin or everyone
  {CLI} uninstall <app> [--scope s]
  {CLI} fork <app> <name> [--owner <owner>]
                                       a new app from <app>'s code, with data of its own
  {CLI} secret set <app> <name> --hosts a,b [--scope user|global|<group>]
                                       set an app's secret, read from stdin: it's sent only to those hosts, and
                                       never read back
  {CLI} secret list [--scope s]        the secrets you may set: names and hosts, never values
  {CLI} secret remove <app> <name> [--scope s]
  {CLI} secret request <app> <name> --hosts a,b [--why text] [--scope s]
                                       ask the person to set a secret: a link to a sheet where they type it, so
                                       you never see it; secret status <id> says how it went
  {CLI} secret requests                the requests waiting for you; secret answer <id> [--decline] sets one from stdin
  {CLI} app delete|restore|erase <app> delete an app you own (to Recently Deleted), restore it, or erase it for good
  {CLI} schedules                      every schedule of the apps installed for you: when, as whom, and how it last went
  {CLI} schedule on|off <app> <name> [--scope user|global|<group>]
                                       switch an app's schedule on (it then runs as you) or off
  {CLI} <app>                          an app's operations
  {CLI} <app> <op> [words] [--input value]
                                       run an operation, e.g. {CLI} board post \"hi there\" --topic x; --help says
                                       what it takes. Required inputs may be words, lists repeat a flag, inputs not
                                       given are read as JSON from stdin, --file <input>=<path> gives a file and
                                       --out <path> saves what it gives back. --server <name> runs it there
  {CLI} update                         test and build main's tip on this machine and the Macs, and run it everywhere
  {CLI} release <vX.Y.Z>               the same, signed off and published on GitHub for the installer
  {CLI} publish [binary]               sign and publish a build you made; every node on its platform updates
  {CLI} blob put <file>                keep a file as a blob; prints its hash
  {CLI} blob get <hash> <file>         get a blob from this server or another
  {CLI} version                        this build
  {CLI} mcp                            serve MCP over stdio, for agents

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
        ["mcp"] => mcp()?,
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
            let r = post("/v1/pair", json!({ "new_user": new_user }))?;
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
        ["deploy", app] if flags.contains_key("bundle") => {
            let bundle = flags["bundle"].as_str().context("give --bundle <hash>")?;
            post("/v1/deploy", json!({ "app": app, "bundle": bundle }))?;
            println!("Deployed {app} ({bundle}) again. Every server runs it now.");
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
            if let Some(b) = flags.get("bundle").and_then(Value::as_str) {
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
                    let mut v = String::new();
                    std::io::stdin().read_line(&mut v)?;
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
                println!("{}  {}/{}  → {}", s["scope"].as_str().unwrap_or(""), s["app"].as_str().unwrap_or(""), s["name"].as_str().unwrap_or(""), hosts.join(", "));
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
        ["blob", "put", file] => {
            let bytes = std::fs::read(file).with_context(|| format!("read {file}"))?;
            println!("{}", request(agent().post(format!("http://{}/v1/blobs", addr())).send(&bytes[..]))?["blob"].as_str().unwrap_or(""));
        }
        ["blob", "get", hash, file] => {
            let mut resp = agent().get(format!("http://{}/v1/blobs/{hash}", addr())).call().with_context(|| format!("no node is running here: start one with {CLI} serve"))?;
            if !resp.status().is_success() {
                bail!("{}", resp.body_mut().read_json::<Value>()?["error"].as_str().unwrap_or("failed"));
            }
            std::fs::write(file, resp.body_mut().with_config().limit(1 << 30).read_to_vec()?)?;
        }
        ["version"] if flags.contains_key("json") => println!("{}", json!({ "commit": one::brand::BUILD, "target": one::brand::TARGET, "committed": one::brand::COMMITTED })),
        ["version"] => println!("{} {} ({})", NAME, one::brand::BUILD, one::brand::TARGET),
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
    println!("Published {} for {}. Every node on that platform updates itself now.", r["commit"].as_str().unwrap_or(""), r["target"].as_str().unwrap_or(""));
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
    // A driver of the same name is listed too, without an owner: the app is still to be made.
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

/// Whether a node runs here, and which install it's in, for the bare command.
fn connection() -> String {
    let Ok(state) = get("/v1/state") else { return format!("No node is running here: start one with {CLI} serve.") };
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
    let mut resp = resp.with_context(|| format!("no node is running here: start one with {CLI} serve"))?;
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
    println!("\n{}  {} entries", i["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("(joining)"), i["entries"]);
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
/// drivers it starts need Homebrew's and the user's own tools.
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

#[tokio::main]
async fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn,iroh=error,noq_proto=error".into()))
        .init();
    let dir = one::home();
    let (node, router) = Node::start(&dir, one::builtin::all()).await?;
    let listener = tokio::net::TcpListener::bind(addr()).await.with_context(|| format!("listen on {}", addr()))?;
    tracing::info!("{NAME} node {} serving {} from {}", &one::entry::hex(&node.id())[..8], addr(), dir.display());
    let http = axum::serve(listener, api::router(node));
    tokio::select! {
        r = http => r?,
        _ = tokio::signal::ctrl_c() => {}
    }
    router.shutdown().await.ok();
    Ok(())
}

/// MCP over stdio: every app operation is a tool, every app's instructions are the server's.
fn mcp() -> Result<()> {
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
                "capabilities": { "tools": {}, "resources": {} },
                "serverInfo": { "name": CLI, "title": NAME, "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions(),
                })
            }),
            "ping" => Ok(json!({})),
            "tools/list" => tools().map(|mut t| {
                t.extend(mcp_apps::tools());
                json!({ "tools": t })
            }),
            "tools/call" => Ok(mcp_apps::call(msg["params"]["name"].as_str().unwrap_or(""), &msg["params"]["arguments"])
                .unwrap_or_else(|| call_tool(&msg["params"], agent.as_deref()))),
            // Apps' widgets and screens, as MCP Apps (src/mcp_apps.rs).
            "resources/list" => Ok(json!({ "resources": mcp_apps::resources() })),
            "resources/read" => mcp_apps::read(msg["params"]["uri"].as_str().unwrap_or("")),
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

fn instructions() -> String {
    let mut text = format!("{NAME} is this user's apps and agents, shared across their machines. Every tool is also a CLI command ({CLI} <app> <op>).");
    if let Ok(apps) = get("/v1/apps") {
        for app in apps.as_array().into_iter().flatten() {
            if let Some(i) = app["instructions"].as_str().filter(|i| !i.is_empty()) {
                text.push_str(&format!("\n\n## {}\n\n{i}", app["title"].as_str().unwrap_or("")));
            }
        }
    }
    text
}

/// Secrets are the node's, not an app's: agents set, replace and delete them, and list their names, never values.
fn secret_tools() -> Vec<Value> {
    let scope = json!({ "type": "string", "description": "user (yours, the default), global, or a group's name or id" });
    vec![
        json!({ "name": "secrets_set", "description": "Set (or replace) a secret of an app: it puts {{name}} in a request's headers or body and the node fills it in, only for these hosts. Nobody can read it back.", "inputSchema": { "type": "object", "required": ["app", "name", "value", "hosts"], "properties": { "app": { "type": "string" }, "name": { "type": "string" }, "value": { "type": "string" }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names it's sent to, like api.example.com" }, "scope": scope } } }),
        json!({ "name": "secrets_list", "description": "The secrets you may set: app, name and hosts, never values.", "inputSchema": { "type": "object", "properties": { "scope": scope } }, "annotations": { "readOnlyHint": true } }),
        json!({ "name": "secrets_remove", "annotations": { "destructiveHint": true }, "description": "Delete a secret of an app.", "inputSchema": { "type": "object", "required": ["app", "name"], "properties": { "app": { "type": "string" }, "name": { "type": "string" }, "scope": scope } } }),
        json!({ "name": "secrets_request", "description": "Ask the person to set a secret of an app, instead of handling its value yourself: returns a link to give them, which opens a sheet in the app where they type it (and their Settings lists it as a request). You never see the value; poll secrets_status with the id until it's set or declined.", "inputSchema": { "type": "object", "required": ["app", "name", "hosts"], "properties": { "app": { "type": "string" }, "name": { "type": "string" }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names it's sent to, like api.example.com" }, "why": { "type": "string", "description": "one line the person sees: what it's for and where to find it" }, "scope": scope } } }),
        json!({ "name": "secrets_status", "description": "How a secret request went: waiting, set, declined or expired, and who answered. Never the value.", "inputSchema": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } }, "annotations": { "readOnlyHint": true } }),
    ]
}

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
    let mut tools = secret_tools();
    for app in apps.as_array().into_iter().flatten() {
        let name = app["name"].as_str().unwrap_or("");
        // Operations starting with `_` are for the node, such as a driver's `_screen`.
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

fn call_tool(params: &Value, agent: Option<&str>) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let mut input = params["arguments"].as_object().cloned().unwrap_or_default();
    let failed = |text: String| json!({ "isError": true, "content": [{ "type": "text", "text": text }] });
    let Some((app, op)) = name.split_once('_') else { return failed(format!("no tool {name}")) };
    if app == "secrets" {
        return match secret_tool(op, &input, agent) {
            Ok(ok) => json!({ "content": ops::mcp_content(&ok) }),
            Err(e) => failed(format!("{e:#}")),
        };
    }
    let server = input.remove("server").or(input.remove("device"));
    let confirmed = input.remove("confirm") == Some(json!(true));
    if let Some((_, spec)) = ops::find(app, op) {
        if spec["confirm"] == true && !confirmed {
            return failed(format!("{name} can't be undone: ask the person first, then call it again with confirm: true"));
        }
        if let Err(e) = ops::mcp_files(&spec, &mut input) {
            return failed(format!("{e:#}"));
        }
    }
    match post("/v1/call", json!({ "server": server, "app": app, "op": op, "input": input, "agent": agent, "client": "mcp" })) {
        Ok(r) => json!({ "content": ops::mcp_content(&r["ok"]) }),
        Err(e) => failed(format!("{e:#}")),
    }
}
