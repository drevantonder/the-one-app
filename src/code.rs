//! Finding tools across every app (`<cli> search`, MCP's `search`), with the exact way to call each and its
//! app's guide, so the long guides come with a search instead of the MCP instructions; and code mode: scripts that call
//! them (`<cli> run`, MCP's `execute`), run by OpenCode's interpreter under the pinned Bun (`src/codemode/run.js`).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use one::config::CLI;
use one::skills::{Mode, path, ts};
use serde_json::{Value, json};

use crate::{get, tools};

/// Who a search answers: someone at the command line, or an MCP client in one of its modes.
#[derive(Clone, Copy)]
pub enum For {
    Cli,
    Mcp(Mode),
}

/// Every app installed for this user that has tools, as `GET /v1/apps` gives them, with each tool's input as
/// MCP takes it (files explained, `device` and `confirm` where they apply), and the Vault's tools as an app.
pub fn catalog() -> Result<Vec<Value>> {
    Ok(catalogs()?.0)
}

/// What a search looks through: `catalog`'s apps, then the apps the person could add, which have no tools for
/// them yet: those not added where they are, nor deleted.
pub fn catalogs() -> Result<(Vec<Value>, Vec<Value>)> {
    let mut apps = vec![one::skills::vault_app(), one::skills::backups_app(), one::skills::groups_app(), one::skills::devices_app()];
    let mut addable = Vec::new();
    for mut app in get("/v1/apps")?.as_array().cloned().unwrap_or_default() {
        let tools: Vec<Value> = app["ops"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|tool| !tool["name"].as_str().unwrap_or("").starts_with('_'))
            .map(|tool| {
                let mut tool = tool.clone();
                tool["input"] = tools::mcp_schema(&app, &tool);
                tool
            })
            .collect();
        if !tools.is_empty() {
            app["ops"] = json!(tools);
            apps.push(app);
        } else if app["ops"].is_null() && app["deleted"].is_null() && app["title"].is_string() {
            addable.push(app);
        }
    }
    Ok((apps, addable))
}

/// Words a search ignores.
const FILLER: &[&str] = &[
    "a", "an", "the", "my", "me", "i", "to", "for", "of", "in", "on", "at", "and", "or", "with", "from", "it", "is", "are", "all", "some", "how", "do", "can", "what", "this", "that",
    "please", "using", "use", "here", "there", "else", "which", "any", "your", "you", "be", "was", "has", "have", "just", "also",
];

/// A word as a search compares it: lower case, without a plural's or a verb's ending.
fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    for end in ["ing", "es", "s", "ed"] {
        // A plural's s even on a short word: keys is key, apps is app.
        if w.len() > end.len() + if end == "s" { 2 } else { 3 }
            && let Some(s) = w.strip_suffix(end)
        {
            return s.to_string();
        }
    }
    w
}

fn words(text: &str) -> Vec<String> {
    // A letter alone is what's left of what's or app's.
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1 && !FILLER.contains(&w.to_lowercase().as_str())).map(stem).collect()
}

/// Whether `w`, a query's word, is in `list`: either may be the other's stem (devices is devic, device is device).
fn has(list: &[String], w: &str) -> bool {
    list.iter().any(|x| x == w || (w.len() > 3 && x.starts_with(w)) || (x.len() > 3 && w.starts_with(x.as_str())))
}

/// How well a tool matches, how many of the query's words its name, its app's (or what the app is for) or its
/// description has, and whether its name has one: its name counts most, then its app's name, its own description, and
/// last what its app is for and its guide, which alone never makes a match; so a word in an app's one line doesn't put
/// every tool of that app above the one whose own description has it.
fn score(app: &Value, tool: &Value, query: &[String]) -> (u32, usize, bool) {
    let name = words(tool["name"].as_str().unwrap_or(""));
    let app_name = words(&format!("{} {}", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or("")));
    let app_about = words(app["description"].as_str().unwrap_or(""));
    let about = words(tool["description"].as_str().unwrap_or(""));
    let guide = words(app["instructions"].as_str().unwrap_or(""));
    let score = query.iter().map(|w| if has(&name, w) { 6 } else { 0 } + if has(&app_name, w) { 4 } else { 0 } + if has(&about, w) { 3 } else { 0 } + if has(&app_about, w) || has(&guide, w) { 1 } else { 0 }).sum();
    (score, query.iter().filter(|w| has(&name, w) || has(&app_name, w) || has(&app_about, w) || has(&about, w)).count(), query.iter().any(|w| has(&name, w)))
}

/// How well an app the person could add matches, by its name and what it says it's for, as `score` does.
fn score_app(app: &Value, query: &[String]) -> (u32, usize) {
    let name = words(&format!("{} {}", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or("")));
    let about = words(&format!("{} {}", app["description"].as_str().unwrap_or(""), app["tagline"].as_str().unwrap_or("")));
    let score = query.iter().map(|w| if has(&name, w) { 4 } else { 0 } + if has(&about, w) { 2 } else { 0 }).sum();
    (score, query.iter().filter(|w| has(&name, w) || has(&about, w)).count())
}

/// The exact way to call a tool, for whoever asked.
fn call(app: &Value, tool: &Value, how: For) -> String {
    let (a, o) = (app["name"].as_str().unwrap_or(""), tool["name"].as_str().unwrap_or(""));
    match how {
        For::Mcp(Mode::Code) => format!("{}({})", path(a, o), ts(&tool["input"])),
        For::Mcp(Mode::All) => format!("{a}_{o} {}", ts(&tool["input"])),
        For::Cli => {
            let schema = &tool["input"];
            let required: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            // The inputs words fill, as --help shows them; the rest are flags.
            let words: Vec<&str> = match tool["words"].as_array() {
                Some(w) => w.iter().filter_map(Value::as_str).collect(),
                None => required.clone(),
            };
            let mut line = format!("{CLI} {a} {o}");
            for w in &words {
                line.push_str(&match required.contains(w) {
                    true => format!(" <{}>", w.replace('_', "-")),
                    false => format!(" [<{}>]", w.replace('_', "-")),
                });
            }
            for r in required.iter().filter(|r| !words.contains(r)) {
                line.push_str(&format!(" --{} <{}>", r.replace('_', "-"), r.replace('_', "-")));
            }
            for (k, v) in schema["properties"].as_object().into_iter().flatten().filter(|(k, _)| !required.contains(&k.as_str()) && !words.contains(&k.as_str())) {
                match v["type"] == "boolean" {
                    true => line.push_str(&format!(" [--{}]", k.replace('_', "-"))),
                    false => line.push_str(&format!(" [--{} {}]", k.replace('_', "-"), ts(v))),
                }
            }
            line
        }
    }
}

/// About how much of a search's answer the matches may take, in characters, before their descriptions are cut short.
const MATCHES: usize = 6_000;

/// Descriptions that fit in `room` characters between them: as written when they do, else each cut to its first
/// sentence, else to an equal share of the room.
fn fit(described: &[&str], room: usize) -> Vec<String> {
    let total = |d: &[String]| d.iter().map(|s| s.chars().count()).sum::<usize>();
    let whole: Vec<String> = described.iter().map(|d| d.to_string()).collect();
    if total(&whole) <= room {
        return whole;
    }
    let mut first: Vec<String> = described.iter().map(|d| format!("{}.", d.split_once(". ").map_or(*d, |(first, _)| first).trim_end_matches('.'))).collect();
    let share = room / first.len().max(1);
    if total(&first) > room {
        for d in first.iter_mut().filter(|d| d.chars().count() > share) {
            *d = format!("{}...", d.chars().take(share.saturating_sub(3)).collect::<String>().trim_end());
        }
    }
    first
}

/// The tools that best match `query`, best first, each with how to call it and what it does, then the guide of
/// each app they're in; and the apps the person could add (`addable`) that match as well. A match is one whose name,
/// app or description has most of the query's words, and as many as the best: fewer is a guess, not an answer.
pub fn search(apps: &[Value], addable: &[Value], query: &str, limit: usize, how: For) -> String {
    // What people call the devices their One App runs on, and keeping something.
    let query: Vec<String> = words(query)
        .into_iter()
        .map(|w| match w.as_str() {
            "server" | "machine" | "box" => "device".into(),
            "save" | "store" | "remember" => "keep".into(),
            _ => w,
        })
        .collect();
    let enough = |covers: usize| covers > 0 && covers * 2 > query.len().max(1);
    let mut found: Vec<(u32, usize, usize, &Value, &Value)> = Vec::new();
    let mut named = Vec::new();
    for (i, app) in apps.iter().enumerate() {
        for tool in app["ops"].as_array().into_iter().flatten() {
            let (s, covers, by_name) = score(app, tool, &query);
            if enough(covers) {
                found.push((s, covers, i, app, tool));
                named.push(by_name);
            }
        }
    }
    // Those that have as many of the words as the best, or one fewer with one in their name, best first.
    let best = found.iter().map(|f| f.1).max().unwrap_or(0);
    let mut named = named.into_iter();
    found.retain(|f| named.next().is_some_and(|by_name| f.1 == best || (by_name && f.1 + 1 == best)));
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(&b.2)));
    found.truncate(limit.max(1));
    // The apps to add are offered when they match at least as well as what's here; when nothing here does, any that
    // has one of the words, since all there is of them is a line on what they're for.
    let fits = |c: usize| match found.is_empty() {
        true => c > 0,
        false => enough(c) && c >= best,
    };
    let mut could: Vec<(u32, &Value)> = addable.iter().filter_map(|a| Some(score_app(a, &query)).filter(|(_, c)| fits(*c)).map(|(s, _)| (s, a))).collect();
    could.sort_by(|a, b| b.0.cmp(&a.0));
    could.truncate(5);
    let add = |app: &Value| {
        let name = app["name"].as_str().unwrap_or("");
        let how = match how {
            For::Cli => format!("{CLI} apps add {name} user"),
            For::Mcp(Mode::All) => format!("apps_add {{ app: \"{name}\", scope: \"user\" }}"),
            For::Mcp(Mode::Code) => format!("tools.apps.add({{ app: \"{name}\", scope: \"user\" }})"),
        };
        format!("- {} ({name}): {}. {how} adds it for the person.", app["title"].as_str().unwrap_or(name), app["description"].as_str().unwrap_or("").trim().trim_end_matches('.'))
    };
    let adds = match could.is_empty() {
        true => String::new(),
        false => format!("Not added yet, and may do it:\n{}", could.iter().map(|(_, a)| add(a)).collect::<Vec<_>>().join("\n")),
    };
    if found.is_empty() {
        if !adds.is_empty() {
            return format!("Nothing added does that.\n\n{adds}");
        }
        let names: Vec<&str> = apps.iter().filter_map(|a| a["name"].as_str()).collect();
        return format!("Nothing matches. Try other words, or an app's name: {}. If no app does it yet, make one: load the skill {}-build.", names.join(", "), one::skills::server_name());
    }
    let mut text = String::new();
    let mut shown: Vec<&Value> = Vec::new();
    let calls: Vec<String> = found.iter().map(|f| call(f.3, f.4, how)).collect();
    // What the descriptions may take of the matches' budget: each call whole, since an agent can't guess its inputs,
    // and the descriptions cut (to their first sentence, then shorter) when they don't fit.
    let room = MATCHES.saturating_sub(calls.iter().map(|c| c.chars().count() + 4).sum::<usize>());
    let about = fit(&found.iter().map(|f| f.4["description"].as_str().unwrap_or("").trim()).collect::<Vec<_>>(), room);
    for (i, (_, _, _, app, tool)) in found.iter().enumerate() {
        let mark = match (tool["reads"] == true, tool["confirm"] == true) {
            (true, _) => " It only reads.",
            (_, true) => match how {
                For::Cli => " It can't be undone: give --confirm once the person agreed.",
                For::Mcp(_) => " It can't be undone: give confirm: true once the person agreed.",
            },
            _ => "",
        };
        text.push_str(&format!("{}\n  {}{mark}\n\n", calls[i], about[i]));
        if !shown.iter().any(|a| a["name"] == app["name"]) {
            shown.push(app);
        }
    }
    for app in shown {
        if let Some(i) = app["instructions"].as_str().filter(|i| !i.is_empty()) {
            text.push_str(&format!("## {} ({})\n\n{}\n\n", app["title"].as_str().unwrap_or(""), app["name"].as_str().unwrap_or(""), i.trim()));
        }
    }
    if !adds.is_empty() {
        text.push_str(&adds);
    }
    text.trim_end().to_string()
}

/// MCP's `search` tool.
pub fn search_tool(mode: Mode) -> Value {
    let calls = match mode {
        Mode::Code => "the exact call for a script (tools.<app>.<tool>({ ... }))",
        Mode::All => "the tool's name and inputs",
    };
    json!({
        "name": "search",
        "description": format!("Find the tools for a task in every app: give a few words, like \"unread email\" or \"post to the board\". Answers the best matches, each with {calls} and what it does, then the guide of each app they're in. Search before guessing, and to learn an app."),
        "inputSchema": { "type": "object", "required": ["query"], "properties": {
            "query": { "type": "string", "description": "a few words for the task, or an app's name" },
            "limit": { "type": "integer", "description": "how many tools, 8 if not given" },
        } },
        "annotations": one::skills::hints(true, false),
    })
}

/// The tools `<cli> mcp --code` serves, of every tool there is: search and execute, the Vault's request (and how it
/// went), and the tools of the apps every agent needs from the start (the config's `always_on`).
pub fn code_tools(all: Vec<Value>) -> Vec<Value> {
    let direct = |t: &Value| {
        let name = t["name"].as_str().unwrap_or("");
        matches!(name, "deploy" | "vault_request" | "vault_status") || one::config::ALWAYS_ON.iter().any(|a| name.strip_prefix(a).is_some_and(|tool| tool.starts_with('_')))
    };
    [vec![search_tool(Mode::Code), execute_tool()], all.into_iter().filter(direct).collect()].concat()
}

/// A script's limits: ten minutes, a hundred calls and 60 KB back.
const LIMITS: (u64, u64, u64) = (600_000, 100, 60_000);

/// MCP's `execute` tool.
pub fn execute_tool() -> Value {
    json!({
        "name": "execute",
        "description": format!("Run a short script that calls tools, and get back what it returns. It's JavaScript without imports, timers or network: call tools as `await tools.<app>.<tool>({{ ... }})` (search gives each exact call), use plain JavaScript between them, and `return` what you want back; console.log lines come back too. Calls run as the person, as any tool does: what sends or can't be undone needs `confirm: true` in its input, given once the person agreed. Up to {} calls, {} minutes and {} KB back.", LIMITS.1, LIMITS.0 / 60_000, LIMITS.2 / 1000),
        "inputSchema": { "type": "object", "required": ["code"], "properties": { "code": { "type": "string", "description": "the script" } } },
        "annotations": one::skills::hints(false, false),
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

/// Runs a script: each call it makes is checked against its tool's inputs and run as the person, as an MCP call
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
        for tool in app["ops"].as_array().into_iter().flatten() {
            tools.push(json!({ "app": name, "tool": tool["name"], "description": tool["description"], "input": tool["input"], "confirm": tool["confirm"] == true }));
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
            let (app, tool) = (msg["app"].as_str().unwrap_or(""), msg["tool"].as_str().unwrap_or(""));
            let spec = apps.iter().find(|a| a["name"] == app).and_then(|a| Some((a.clone(), a["ops"].as_array()?.iter().find(|o| o["name"] == tool)?.clone())));
            let input = msg["input"].clone();
            let answer = (|| {
                let spec = spec.with_context(|| format!("no tool {app}_{tool}"))?;
                anyhow::ensure!(input.is_object() || input.is_null(), "a tool's input is an object, not {input}");
                crate::dispatch(&format!("{app}_{tool}"), input.as_object().cloned().unwrap_or_default(), agent, client, Some(spec))
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
