//! Finding operations across every app (`<cli> search`, MCP's `search`), with the exact way to call each and its
//! app's guide, so the long guides come with a search instead of the MCP instructions; and code mode: scripts that call
//! them (`<cli> run`, MCP's `execute`), run by OpenCode's interpreter under the pinned Bun (`src/codemode/run.js`).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use one::config::CLI;
use one::skills::{Mode, path, ts};
use serde_json::{Value, json};

use crate::{get, ops};

/// Who a search answers: someone at the command line, or an MCP client in one of its modes.
#[derive(Clone, Copy)]
pub enum For {
    Cli,
    Mcp(Mode),
}

/// Every app installed for this user that has operations, as `GET /v1/apps` gives them, with each operation's input as
/// MCP takes it (files explained, `server` and `confirm` where they apply), and the Vault's tools as an app.
pub fn catalog() -> Result<Vec<Value>> {
    let mut apps = vec![one::skills::vault_app(), one::skills::backups_app(), one::skills::groups_app()];
    for mut app in get("/v1/apps")?.as_array().cloned().unwrap_or_default() {
        let ops: Vec<Value> = app["ops"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|op| !op["name"].as_str().unwrap_or("").starts_with('_'))
            .map(|op| {
                let mut op = op.clone();
                op["input"] = ops::mcp_schema(&app, &op);
                op
            })
            .collect();
        if !ops.is_empty() {
            app["ops"] = json!(ops);
            apps.push(app);
        }
    }
    Ok(apps)
}

/// Words a search ignores.
const FILLER: &[&str] = &[
    "a", "an", "the", "my", "me", "i", "to", "for", "of", "in", "on", "at", "and", "or", "with", "from", "it", "is", "are", "all", "some", "how", "do", "can", "what", "this", "that",
    "please", "using", "use",
];

/// A word as a search compares it: lower case, without a plural's or a verb's ending.
fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    for end in ["ing", "es", "s", "ed"] {
        if w.len() > end.len() + 3
            && let Some(s) = w.strip_suffix(end)
        {
            return s.to_string();
        }
    }
    w
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && !FILLER.contains(&w.to_lowercase().as_str())).map(stem).collect()
}

/// How well an operation matches: its name counts most, then its app's, its description, and its app's guide.
fn score(app: &Value, op: &Value, query: &[String]) -> u32 {
    let name = words(op["name"].as_str().unwrap_or(""));
    let app_name = words(&format!("{} {}", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or("")));
    let about = words(op["description"].as_str().unwrap_or(""));
    let guide = words(&format!("{} {}", app["description"].as_str().unwrap_or(""), app["instructions"].as_str().unwrap_or("")));
    let has = |list: &[String], w: &str| list.iter().any(|x| x == w || (w.len() > 3 && x.starts_with(w)));
    query.iter().map(|w| if has(&name, w) { 6 } else { 0 } + if has(&app_name, w) { 4 } else { 0 } + if has(&about, w) { 2 } else { 0 } + if has(&guide, w) { 1 } else { 0 }).sum()
}

/// The exact way to call an operation, for whoever asked.
fn call(app: &Value, op: &Value, how: For) -> String {
    let (a, o) = (app["name"].as_str().unwrap_or(""), op["name"].as_str().unwrap_or(""));
    match how {
        For::Mcp(Mode::Code) => format!("{}({})", path(a, o), ts(&op["input"])),
        For::Mcp(Mode::All) => format!("{a}_{o} {}", ts(&op["input"])),
        For::Cli => {
            let schema = &op["input"];
            let required: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let mut line = format!("{CLI} {a} {o}");
            for r in &required {
                line.push_str(&format!(" <{}>", r.replace('_', "-")));
            }
            for (k, v) in schema["properties"].as_object().into_iter().flatten().filter(|(k, _)| !required.contains(&k.as_str())) {
                match v["type"] == "boolean" {
                    true => line.push_str(&format!(" [--{}]", k.replace('_', "-"))),
                    false => line.push_str(&format!(" [--{} {}]", k.replace('_', "-"), ts(v))),
                }
            }
            line
        }
    }
}

/// The operations that best match `query`, best first, each with how to call it and what it does, then the guide of
/// each app they're in.
pub fn search(apps: &[Value], query: &str, limit: usize, how: For) -> String {
    let query = words(query);
    let mut found: Vec<(u32, usize, &Value, &Value)> = Vec::new();
    for (i, app) in apps.iter().enumerate() {
        for op in app["ops"].as_array().into_iter().flatten() {
            let s = score(app, op, &query);
            if s > 0 {
                found.push((s, i, app, op));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.truncate(limit.max(1));
    if found.is_empty() {
        let names: Vec<&str> = apps.iter().filter_map(|a| a["name"].as_str()).collect();
        return format!("Nothing matches. Try other words, or an app's name: {}.", names.join(", "));
    }
    let mut text = String::new();
    let mut shown: Vec<&Value> = Vec::new();
    for (_, _, app, op) in &found {
        let mark = match (op["reads"] == true, op["confirm"] == true) {
            (true, _) => " It only reads.",
            (_, true) => match how {
                For::Cli => " It can't be undone: give --confirm once the person agreed.",
                For::Mcp(_) => " It can't be undone: give confirm: true once the person agreed.",
            },
            _ => "",
        };
        text.push_str(&format!("{}\n  {}{mark}\n\n", call(app, op, how), op["description"].as_str().unwrap_or("").trim()));
        if !shown.iter().any(|a| a["name"] == app["name"]) {
            shown.push(app);
        }
    }
    for app in shown {
        if let Some(i) = app["instructions"].as_str().filter(|i| !i.is_empty()) {
            text.push_str(&format!("## {} ({})\n\n{}\n\n", app["title"].as_str().unwrap_or(""), app["name"].as_str().unwrap_or(""), i.trim()));
        }
    }
    text.trim_end().to_string()
}

/// MCP's `search` tool.
pub fn search_tool(mode: Mode) -> Value {
    let calls = match mode {
        Mode::Code => "the exact call for a script (tools.<app>.<op>({ ... }))",
        Mode::All => "the tool's name and inputs",
    };
    json!({
        "name": "search",
        "description": format!("Find the operations for a task in every app: give a few words, like \"unread email\" or \"post to the board\". Answers the best matches, each with {calls} and what it does, then the guide of each app they're in. Search before guessing, and to learn an app."),
        "inputSchema": { "type": "object", "required": ["query"], "properties": {
            "query": { "type": "string", "description": "a few words for the task, or an app's name" },
            "limit": { "type": "integer", "description": "how many operations, 8 if not given" },
        } },
        "annotations": { "readOnlyHint": true },
    })
}

/// The tools `<cli> mcp --code` serves, of every tool there is: search and execute, the Vault's request (and how it
/// went), and the tools of the apps every agent needs from the start (the config's `always_on`).
pub fn code_tools(all: Vec<Value>) -> Vec<Value> {
    let direct = |t: &Value| {
        let name = t["name"].as_str().unwrap_or("");
        matches!(name, "vault_request" | "vault_status") || one::config::ALWAYS_ON.iter().any(|a| name.strip_prefix(a).is_some_and(|op| op.starts_with('_')))
    };
    [vec![search_tool(Mode::Code), execute_tool()], all.into_iter().filter(direct).collect()].concat()
}

/// A script's limits, as the old PTC App's were: ten minutes, a hundred calls and 60 KB back.
const LIMITS: (u64, u64, u64) = (600_000, 100, 60_000);

/// MCP's `execute` tool.
pub fn execute_tool() -> Value {
    json!({
        "name": "execute",
        "description": format!("Run a short script that calls operations, and get back what it returns. It's JavaScript without imports, timers or network: call operations as `await tools.<app>.<op>({{ ... }})` (search gives each exact call), use plain JavaScript between them, and `return` what you want back; console.log lines come back too. Calls run as the person, as any tool does: what sends or can't be undone needs `confirm: true` in its input, given once the person agreed. Up to {} calls, {} minutes and {} KB back.", LIMITS.1, LIMITS.0 / 60_000, LIMITS.2 / 1000),
        "inputSchema": { "type": "object", "required": ["code"], "properties": { "code": { "type": "string", "description": "the script" } } },
    })
}

/// The runner's files, and the npm packages it needs, pinned by their lockfile.
const RUNNER: &[(&str, &str)] = &[("package.json", include_str!("codemode/package.json")), ("bun.lock", include_str!("codemode/bun.lock")), ("run.js", include_str!("codemode/run.js"))];

/// The folder the runner runs in, its packages installed (once per version of its files) with the pinned Bun.
fn runner() -> Result<(PathBuf, PathBuf)> {
    use sha2::Digest;
    let bun = one::deploy::bun()?;
    let mut hash = sha2::Sha256::new();
    for (path, text) in RUNNER {
        hash.update(format!("{path}\0{}\0{text}", text.len()));
    }
    let hash = data_encoding::HEXLOWER.encode(&hash.finalize());
    let dir = one::cache().join(format!("codemode-{}", &hash[..16]));
    if dir.join("node_modules/@opencode/codemode").is_dir() {
        return Ok((bun, dir));
    }
    // Made beside it, then moved into place, so two at once can't see half of one.
    let making = dir.with_extension(format!("{}", std::process::id()));
    std::fs::create_dir_all(&making)?;
    for (path, text) in RUNNER {
        std::fs::write(making.join(path), text)?;
    }
    let out = Command::new(&bun)
        .args(["install", "--frozen-lockfile"])
        .current_dir(&making)
        .env("BUN_INSTALL_CACHE_DIR", bun.parent().unwrap().join("cache"))
        .env("DO_NOT_TRACK", "1")
        .output()?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&making);
        bail!("installing code mode's packages failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
    if std::fs::rename(&making, &dir).is_err() {
        let _ = std::fs::remove_dir_all(&making);
    }
    Ok((bun, dir))
}

/// Runs a script: each call it makes is checked against its operation's inputs and run as the person, as an MCP call
/// is, what sends or can't be undone only with confirm: true. Gives back code mode's result: `{ ok, value | error,
/// logs, truncated, toolCalls }`.
pub fn execute(script: &str, agent: Option<&str>, client: &str) -> Result<Value> {
    anyhow::ensure!(!script.trim().is_empty(), "give the script as code");
    let apps = catalog()?;
    let mut tools = Vec::new();
    let mut descriptions = serde_json::Map::new();
    for app in &apps {
        let name = app["name"].as_str().unwrap_or("");
        descriptions.insert(name.into(), app["description"].clone());
        for op in app["ops"].as_array().into_iter().flatten() {
            tools.push(json!({ "app": name, "op": op["name"], "description": op["description"], "input": op["input"], "confirm": op["confirm"] == true }));
        }
    }
    let (bun, dir) = runner()?;
    let mut child = Command::new(bun).arg("run.js").current_dir(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut to = child.stdin.take().unwrap();
    let from = BufReader::new(child.stdout.take().unwrap());
    let limits = json!({ "timeoutMs": LIMITS.0, "maxToolCalls": LIMITS.1, "maxOutputBytes": LIMITS.2 });
    writeln!(to, "{}", json!({ "code": script, "tools": tools, "limits": limits, "descriptions": descriptions }))?;
    // Past its own time limit, with a minute for the last call to come back, it's stopped.
    let pid = child.id() as i32;
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watching = done.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(LIMITS.0 + 60_000));
        if !watching.load(std::sync::atomic::Ordering::SeqCst) {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    });
    let mut result = None;
    for line in from.lines() {
        let Ok(msg) = serde_json::from_str::<Value>(&line?) else { continue };
        if let Some(id) = msg.get("call") {
            let (app, op) = (msg["app"].as_str().unwrap_or(""), msg["op"].as_str().unwrap_or(""));
            let spec = apps.iter().find(|a| a["name"] == app).and_then(|a| a["ops"].as_array()?.iter().find(|o| o["name"] == op)).cloned();
            let input = msg["input"].clone();
            let answer = (|| {
                let spec = spec.with_context(|| format!("no operation {app}_{op}"))?;
                crate::ops::check(&spec["input"], &input, "input")?;
                crate::dispatch(&format!("{app}_{op}"), input.as_object().cloned().unwrap_or_default(), agent, client, Some(spec))
            })();
            let reply = match answer {
                Ok(ok) => json!({ "reply": id, "ok": ok }),
                Err(e) => json!({ "reply": id, "error": format!("{e:#}") }),
            };
            // A script that has stopped listening has finished, or been stopped.
            if writeln!(to, "{reply}").is_err() {
                break;
            }
        } else if let Some(r) = msg.get("done") {
            result = Some(r.clone());
            break;
        }
    }
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(to);
    let _ = child.kill();
    let out = child.wait_with_output()?;
    result.with_context(|| format!("the script stopped without an answer: {}", String::from_utf8_lossy(&out.stderr).trim()))
}

/// A script's result as text for an agent: what it returned, or what went wrong, with its logs.
pub fn shown(result: &Value) -> String {
    let mut text = match result["ok"] == true {
        true => match &result["value"] {
            Value::String(s) => s.clone(),
            v => serde_json::to_string(v).unwrap_or_default(),
        },
        false => format!("{}: {}", result["error"]["kind"].as_str().unwrap_or("Error"), result["error"]["message"].as_str().unwrap_or("")),
    };
    let logs: Vec<String> = result["logs"].as_array().into_iter().flatten().map(|l| l.as_str().map(String::from).unwrap_or_else(|| l.to_string())).collect();
    if !logs.is_empty() {
        text.push_str(&format!("\n\nLogged:\n{}", logs.join("\n")));
    }
    for w in result["warnings"].as_array().into_iter().flatten() {
        text.push_str(&format!("\n\nWarning: {}", w["message"].as_str().unwrap_or("")));
    }
    if result["truncated"] == true {
        text.push_str(&format!("\n\n(cut at {} KB: return less)", LIMITS.2 / 1000));
    }
    text
}
