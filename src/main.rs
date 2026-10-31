use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use one::brand::{CLI, NAME, PORT};
use one::{api, node::Node};
use serde_json::{Map, Value, json};

fn help() -> String {
    format!(
        "{NAME}: your devices as one place for apps and agents.

Usage:
  {CLI} serve                       run this device's node
  {CLI} status                      this device, its spaces and their devices
  {CLI} create <name>               create a space
  {CLI} pair [--new-user]           a pairing code for another of your devices (or a new user's)
  {CLI} join <code>                 join a space with a pairing code
  {CLI} rename <name>               rename this device
  {CLI} remove <device>             remove a device from the space (admins)
  {CLI} apps                        every app and its operations
  {CLI} <app> <op> [--key value]    run an operation, e.g. {CLI} board post --text hi
  {CLI} release [binary]            publish a build; every node on its platform updates (admins)
  {CLI} version                     this build
  {CLI} mcp                         serve MCP over stdio, for agents

Any command takes --space <name> when the device is in more than one space."
    )
}

fn home() -> PathBuf {
    if let Ok(dir) = std::env::var(format!("{}_HOME", CLI.to_uppercase())) {
        return dir.into();
    }
    let base = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| dirs_home().join(".local/share"));
    base.join(CLI)
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| ".".into())
}

fn addr() -> String {
    std::env::var(format!("{}_ADDR", CLI.to_uppercase())).unwrap_or_else(|_| format!("127.0.0.1:{PORT}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = run(args) {
        eprintln!("{CLI}: {err:#}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let (positional, flags) = parse(&args);
    let space = flags.get("space").and_then(Value::as_str).map(str::to_string);
    match positional.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["help"] => println!("{}", help()),
        ["serve"] => serve()?,
        ["mcp"] => mcp()?,
        ["status"] => status(&get("/v1/state")?),
        ["apps"] => {
            for app in get("/v1/apps")?.as_array().into_iter().flatten() {
                println!("{} ({})", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or(""));
                for op in app["ops"].as_array().into_iter().flatten() {
                    println!("  {:<10} {}", op["name"].as_str().unwrap_or(""), op["description"].as_str().unwrap_or(""));
                }
            }
        }
        ["create", name @ ..] if !name.is_empty() => {
            post("/v1/spaces", json!({ "name": name.join(" ") }))?;
            println!("Created {}. Add your other devices with: {CLI} pair", name.join(" "));
        }
        ["pair"] => {
            let r = post("/v1/pair", json!({ "space": space, "new_user": flags.contains_key("new-user") }))?;
            println!("On the other device, run:\n\n  {CLI} join {}\n\nThe pairing code works once, for a week.", r["code"].as_str().unwrap_or(""));
        }
        ["join", code] => {
            post("/v1/join", json!({ "code": code }))?;
            println!("Joined. Syncing now; {CLI} status shows progress.");
        }
        ["remove", device] => {
            let state = get("/v1/state")?;
            let found = state["spaces"].as_array().into_iter().flatten().flat_map(|s| s["devices"].as_array().cloned().unwrap_or_default())
                .find(|d| d["name"].as_str() == Some(device) || d["device"].as_str().is_some_and(|id| id.starts_with(device)))
                .with_context(|| format!("no device called {device}"))?;
            post("/v1/remove", json!({ "space": space, "device": found["device"] }))?;
            println!("Removed {}.", found["name"].as_str().unwrap_or(device));
        }
        ["release", rest @ ..] => {
            // A release is this binary, or one built for another platform: --target <triple> --commit <hash>.
            let path = match rest {
                [path] => std::fs::canonicalize(path)?,
                _ => std::env::current_exe()?,
            };
            // The binary knows what it is; ask it, unless it's for another platform and can't run here.
            let version: Value = std::process::Command::new(&path)
                .args(["version", "--json"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| serde_json::from_slice(&o.stdout).ok())
                .unwrap_or_default();
            let ran_commit = version["commit"].as_str().map(str::to_string);
            let ran_target = version["target"].as_str().map(str::to_string);
            let committed = flags.get("committed").and_then(Value::as_u64).or(version["committed"].as_u64()).context("say when the commit was made with --committed (seconds since 1970)")?;
            let commit = flags.get("commit").and_then(Value::as_str).map(str::to_string).or(ran_commit).context("say which commit this is with --commit")?;
            let target = flags.get("target").and_then(Value::as_str).map(str::to_string).or(ran_target).context("say which platform this is for with --target")?;
            if commit.ends_with("+dirty") {
                bail!("that build has uncommitted changes; commit, build again, then release");
            }
            let r = post("/v1/release", json!({ "space": space, "path": path, "commit": commit, "target": target, "committed": committed }))?;
            println!("Released {} for {}. Every node on that platform updates itself now.", r["commit"].as_str().unwrap_or(""), r["target"].as_str().unwrap_or(""));
        }
        ["version"] if flags.contains_key("json") => println!("{}", json!({ "commit": one::brand::BUILD, "target": one::brand::TARGET, "committed": one::brand::COMMITTED })),
        ["version"] => println!("{} {} ({})", NAME, one::brand::BUILD, one::brand::TARGET),
        ["rename", name @ ..] if !name.is_empty() => {
            post("/v1/device", json!({ "name": name.join(" ") }))?;
            println!("This device is now {}.", name.join(" "));
        }
        [app, op] => {
            let mut input = flags.clone();
            input.remove("space");
            let device = input.remove("device");
            let r = post("/v1/call", json!({ "space": space, "device": device, "app": app, "op": op, "input": input }))?;
            match &r["ok"] {
                ok if ok["content"].is_array() => print_content(app, op, &ok["content"])?,
                Value::String(s) => println!("{s}"),
                Value::Null => {}
                other => println!("{}", serde_json::to_string_pretty(other)?),
            }
        }
        _ => bail!("unknown command; {CLI} help lists them"),
    }
    Ok(())
}

/// Prints an operation's MCP content (`{ content: [...] }`, as a provider passing on an MCP server's answers gives it):
/// its text, and the path of each image, saved to a file.
fn print_content(app: &str, op: &str, content: &Value) -> Result<()> {
    for (i, c) in content.as_array().into_iter().flatten().enumerate() {
        match c["type"].as_str() {
            Some("image") => {
                let ext = c["mimeType"].as_str().and_then(|m| m.strip_prefix("image/")).unwrap_or("img");
                let path = std::env::temp_dir().join(format!("{app}-{op}-{i}.{ext}"));
                std::fs::write(&path, data_encoding::BASE64.decode(c["data"].as_str().unwrap_or("").as_bytes())?)?;
                println!("{}", path.display());
            }
            _ => println!("{}", c["text"].as_str().unwrap_or("")),
        }
    }
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
    println!("{} ({})", state["device"]["name"].as_str().unwrap_or(""), &state["device"]["id"].as_str().unwrap_or("")[..8]);
    let spaces = state["spaces"].as_array().cloned().unwrap_or_default();
    if spaces.is_empty() {
        println!("\nIn no space yet. Create one with {CLI} create <name>, or join with a pairing code.");
    }
    for s in spaces {
        println!("\n{}  {} entries", s["name"].as_str().unwrap_or("(joining)"), s["entries"]);
        for d in s["devices"].as_array().into_iter().flatten() {
            let dot = if d["online"] == true { "●" } else { "○" };
            let me = if d["me"] == true { " (this device)" } else { "" };
            let build = d["build"].as_str().map(|b| format!("  {b}")).unwrap_or_default();
            println!("  {dot} {}{me}{build}", d["name"].as_str().unwrap_or(""));
        }
    }
}

#[tokio::main]
async fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn,iroh=error,noq_proto=error".into()))
        .init();
    let dir = home();
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
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue };
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => Ok(json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": CLI, "title": NAME, "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions(),
            })),
            "ping" => Ok(json!({})),
            "tools/list" => tools().map(|t| json!({ "tools": t })),
            "tools/call" => Ok(call_tool(&msg["params"])),
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

fn instructions() -> String {
    let mut text = format!("{NAME} is this user's apps and agents, shared across their devices. Every tool is also a CLI command ({CLI} <app> <op>).");
    if let Ok(apps) = get("/v1/apps") {
        for app in apps.as_array().into_iter().flatten() {
            if let Some(i) = app["instructions"].as_str().filter(|i| !i.is_empty()) {
                text.push_str(&format!("\n\n## {}\n\n{i}", app["title"].as_str().unwrap_or("")));
            }
        }
    }
    text
}

fn tools() -> Result<Vec<Value>, Value> {
    let apps = get("/v1/apps").map_err(|e| json!({ "code": -32000, "message": e.to_string() }))?;
    let several = get("/v1/state").ok().and_then(|s| s["spaces"].as_array().map(|a| a.len() > 1)).unwrap_or(false);
    let mut tools = Vec::new();
    for app in apps.as_array().into_iter().flatten() {
        let name = app["name"].as_str().unwrap_or("");
        for op in app["ops"].as_array().into_iter().flatten() {
            let mut schema = op["input"].clone();
            if !schema.is_object() {
                schema = json!({ "type": "object", "properties": {} });
            }
            if several {
                schema["properties"]["space"] = json!({ "type": "string", "description": "which space; this device is in several" });
            }
            if app["runs"] == "here" {
                schema["properties"]["device"] = json!({ "type": "string", "description": "which of your devices, by name; default is this one" });
            }
            tools.push(json!({ "name": format!("{name}_{}", op["name"].as_str().unwrap_or("")), "description": op["description"], "inputSchema": schema }));
        }
    }
    Ok(tools)
}

fn call_tool(params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let mut input = params["arguments"].as_object().cloned().unwrap_or_default();
    let space = input.remove("space");
    let device = input.remove("device");
    let Some((app, op)) = name.split_once('_') else {
        return json!({ "isError": true, "content": [{ "type": "text", "text": format!("no tool {name}") }] });
    };
    match post("/v1/call", json!({ "space": space, "device": device, "app": app, "op": op, "input": input })) {
        Ok(r) => match &r["ok"] {
            ok if ok["content"].is_array() => json!({ "content": ok["content"] }),
            Value::String(s) => json!({ "content": [{ "type": "text", "text": s }] }),
            other => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(other).unwrap_or_default() }] }),
        },
        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("{e:#}") }] }),
    }
}
