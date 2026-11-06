//! The MCP server: every app's tool as a tool, with `search` to find them; or in code mode, `search` and `execute`,
//! which runs scripts with them, and the few tools every agent needs from the start. The CLI serves it over stdio
//! (`one mcp`), asking its node as a client does; the node serves it over HTTP at `/mcp`, asking itself in-process as
//! whoever sent the request (`api::client::InProcess`).

pub mod apps;

use anyhow::Result;
use serde_json::{Value, json};

use crate::api::Client;
use crate::config::{CLI, NAME};
use crate::skills::Mode;
use crate::{code, tools};

/// Who an MCP server answers: its client's name (`initialize` says it), credited on its calls, and how it answers.
pub struct Session {
    pub agent: Option<String>,
    pub mode: Mode,
    /// Over stdio, where it tells its client when the tools it lists change; over HTTP it says so in an answer.
    pub stdio: bool,
}

/// Serves MCP over stdio, asking `node`, for an agent on this machine.
pub fn stdio(node: Client, mode: Mode) -> Result<()> {
    use std::io::BufRead;
    let mut session = Session { agent: agent_from_env(), mode, stdio: true };
    for line in std::io::stdin().lock().lines() {
        let Ok(msg) = serde_json::from_str::<Value>(&line?) else { continue };
        let node = node.again();
        // A tool call is answered beside the rest, which go on meanwhile: some wait (a chat's view waiting on a change).
        if msg["method"] == "tools/call" {
            let mut session = Session { agent: session.agent.clone(), ..session };
            std::thread::spawn(move || say(answer(&node, &msg, &mut session)));
            continue;
        }
        say(answer(&node, &msg, &mut session))?;
    }
    Ok(())
}

/// Writes an answer, and that the tools listed changed when they did.
fn say(answered: Option<(Value, bool)>) -> std::io::Result<()> {
    use std::io::Write;
    let Some((reply, changed)) = answered else { return Ok(()) };
    let mut out = std::io::stdout().lock();
    writeln!(out, "{reply}")?;
    if changed {
        writeln!(out, "{}", json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }))?;
    }
    out.flush()
}

/// MCP over Streamable HTTP, at `/mcp`: for a client device over iroh, as its person (the remote MCP server,
/// cloudflare/mcp/, is remote MCP clients' device), or for a program on this machine, as its person. Each POST is
/// answered with JSON; the server sends nothing unasked, so there's no stream to GET. A tool that needs keys it wasn't
/// handed is answered 428 with what it needs, as `/v1/call` is, for its caller to hand them and ask again. Nothing is
/// kept between requests, since the next may reach another node: the session id carries only the client's name,
/// credited on its calls.
pub async fn http(
    axum::extract::State(node): axum::extract::State<crate::node::Node>,
    client: Option<axum::Extension<crate::api::FromClient>>,
    peer: Option<axum::Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    headers: axum::http::HeaderMap,
    body: String,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let elsewhere = client.is_some() || headers.contains_key(axum::http::header::AUTHORIZATION);
    let asks = Client::new(crate::api::client::InProcess::new(node, client.map(|c| c.0), peer.map(|p| p.0), &headers), elsewhere);
    let agent = headers.get("mcp-session-id").and_then(|s| s.to_str().ok()).and_then(|s| s.split_once('.')).and_then(|(_, name)| data_encoding::BASE64URL_NOPAD.decode(name.as_bytes()).ok()).and_then(|n| String::from_utf8(n).ok());
    let Ok(msg) = serde_json::from_str::<Value>(&body) else {
        let error = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "the body isn't JSON" } });
        return (StatusCode::BAD_REQUEST, axum::Json(error)).into_response();
    };
    let initialize = msg["method"] == "initialize";
    let answered = tokio::task::spawn_blocking(move || {
        let mut session = Session { agent, mode: Mode::All, stdio: false };
        let reply = match &msg {
            Value::Array(batch) => Some(json!(batch.iter().filter_map(|m| Some(answer(&asks, m, &mut session)?.0)).collect::<Vec<_>>())).filter(|r| r.as_array().is_some_and(|r| !r.is_empty())),
            msg => answer(&asks, msg, &mut session).map(|(reply, _)| reply),
        };
        (reply, session.agent, asks.needed())
    })
    .await;
    match answered {
        Ok((_, _, Some(needed))) => (StatusCode::PRECONDITION_REQUIRED, axum::Json(needed)).into_response(),
        Ok((None, _, None)) => StatusCode::ACCEPTED.into_response(),
        Ok((Some(reply), agent, None)) => {
            let mut resp = axum::Json(reply).into_response();
            if initialize {
                let session = format!("{:016x}.{}", rand::random::<u64>(), data_encoding::BASE64URL_NOPAD.encode(agent.unwrap_or_default().as_bytes()));
                resp.headers_mut().insert("mcp-session-id", session.parse().expect("base64url and hex are header-safe"));
            }
            resp
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({ "error": err.to_string() }))).into_response(),
    }
}

/// The answer to one MCP message, and whether the tools the session lists changed: None for a notification.
pub fn answer(node: &Client, msg: &Value, session: &mut Session) -> Option<(Value, bool)> {
    let id = msg.get("id")?.clone();
    let mode = session.mode;
    let mut changed = false;
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => Ok({
            if let Some(name) = msg["params"]["clientInfo"]["name"].as_str().filter(|n| !n.trim().is_empty()) {
                session.agent = Some(agent_name(name));
            }
            json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": { "listChanged": session.stdio }, "resources": {}, "extensions": { "io.modelcontextprotocol/skills": {} } },
            "serverInfo": { "name": CLI, "title": NAME, "version": env!("CARGO_PKG_VERSION") },
            "instructions": instructions(node, mode),
            })
        }),
        "ping" => Ok(json!({})),
        "tools/list" => listed(node).map(|t| {
            let mut t = match mode {
                Mode::All => [vec![code::search_tool(mode)], t].concat(),
                Mode::Code => code::code_tools(t),
            };
            t.extend(apps::tools());
            json!({ "tools": t })
        }),
        "tools/call" => Ok({
            let (out, did) = call_and_tell(node, &msg["params"], session);
            changed = did;
            out
        }),
        // Apps' widgets and screens, as MCP Apps (src/mcp/apps.rs).
        "resources/list" => Ok(json!({ "resources": apps::resources() })),
        "resources/read" => match msg["params"]["uri"].as_str().unwrap_or("") {
            uri if uri.starts_with("skill://") => skill_resource(node, uri),
            uri => apps::read(uri),
        },
        // Skills over MCP (SEP-2640): the same skills the agents service writes into harnesses' folders.
        "skills/list" => Ok(json!({ "resultType": "complete", "skills": skills(node).iter().map(skill_entry).collect::<Vec<_>>(), "ttlMs": 60_000, "cacheScope": "session" })),
        "skills/get" => {
            let uri = msg["params"]["uri"].as_str().unwrap_or("");
            skills(node).iter().find(|s| format!("skill://{}/SKILL.md", s.name) == uri).map(|s| json!({ "skill": skill_entry(s) })).ok_or_else(|| json!({ "code": -32602, "message": format!("no skill {uri}") }))
        }
        other => Err(json!({ "code": -32601, "message": format!("unknown method {other}") })),
    };
    let reply = match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    };
    Some((reply, changed))
}

/// Every app's tools by name, as `GET /v1/apps` gives them: what a session's list of tools is made of.
fn tool_names(apps: &[Value]) -> Vec<String> {
    let mut names: Vec<String> = apps.iter().flat_map(|a| a["tools"].as_array().into_iter().flatten().map(move |o| format!("{}_{}", a["name"].as_str().unwrap_or(""), o["name"].as_str().unwrap_or("")))).collect();
    names.sort();
    names
}

/// A tool call, and whether it changed the tools a session lists (adding an app, say): over stdio, the server sends
/// MCP's notification after the answer; over HTTP, which sends nothing unasked, the answer says which tools are new.
fn call_and_tell(node: &Client, params: &Value, session: &Session) -> (Value, bool) {
    let name = params["name"].as_str().unwrap_or("");
    // Search and the core's tools never change which tools there are.
    let apps = (session.mode == Mode::All && name != "search" && crate::core_tools::find(name).is_none()).then(|| node.apps().ok()).flatten();
    let reads = |apps: &[Value]| match name.split_once('_') {
        Some((a, o)) => apps.iter().filter(|x| x["name"] == a).flat_map(|x| x["tools"].as_array().into_iter().flatten()).any(|x| x["name"] == o && x["reads"] == true),
        None => name == "search",
    };
    let before = apps.filter(|a| !reads(a)).map(|a| tool_names(&a));
    let mut out = call_tool(node, params, session);
    let Some(before) = before else { return (out, false) };
    let Ok(after) = node.get("/v1/apps").map(|a| tool_names(a.as_array().map(Vec::as_slice).unwrap_or_default())) else { return (out, false) };
    if after == before {
        return (out, false);
    }
    if session.stdio {
        return (out, true);
    }
    let mut new: Vec<&str> = after.iter().filter(|t| !before.contains(t)).filter_map(|t| t.split_once('_').map(|(a, _)| a)).collect();
    new.dedup();
    if let (Some(first), Some(content)) = (new.first(), out["content"].as_array_mut()) {
        content.push(json!({ "type": "text", "text": format!("New tools for {}: they come with your next session. Until then, `{CLI} {first} <tool>` runs them where `{CLI}` runs.", new.join(", ")) }));
    }
    (out, false)
}

/// The skills for this person: the root first (src/skills.rs).
pub fn skills(node: &Client) -> Vec<crate::skills::Skill> {
    crate::skills::all(&node.apps().unwrap_or_default())
}

/// A skill as `skills/list` lists it: where its SKILL.md is, its frontmatter, and each of its files with its digest.
fn skill_entry(s: &crate::skills::Skill) -> Value {
    use sha2::Digest;
    let files: Vec<Value> = s
        .all_files()
        .iter()
        .map(|(path, text)| json!({ "uri": format!("skill://{}/{path}", s.name), "digest": format!("sha256:{}", data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(text.as_bytes()))), "size": text.len() }))
        .collect();
    json!({ "uri": format!("skill://{}/SKILL.md", s.name), "frontmatter": { "name": s.name, "description": s.description }, "resources": files })
}

/// One of a skill's files, as `resources/read` gives it: `skill://<name>/<path>`.
fn skill_resource(node: &Client, uri: &str) -> Result<Value, Value> {
    let (name, path) = uri.trim_start_matches("skill://").split_once('/').unwrap_or((uri, ""));
    let text = skills(node).into_iter().find(|s| s.name == name).and_then(|s| s.file(path)).ok_or_else(|| json!({ "code": -32002, "message": format!("no resource {uri}") }))?;
    let mime = if path.ends_with(".js") { "text/javascript" } else { "text/markdown" };
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": mime, "text": text }] }))
}

/// The agent running this program, if one is: `<CLI>_AGENT`, else what agent harnesses say about themselves.
pub fn agent_from_env() -> Option<String> {
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

/// The MCP server's instructions (src/skills.rs): the rules first, short enough that every harness keeps all of them.
fn instructions(node: &Client, mode: Mode) -> String {
    // A harness starts its MCP server at login, before the node may be up: the instructions then say less, and a
    // tool call says why it can't.
    let state = node.state().ok();
    crate::skills::instructions(mode, &crate::core_tools::and_apps(node).unwrap_or_default(), state.as_ref().and_then(|s| s.user.as_ref()).map(|u| u.name.as_str()))
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
        "annotations": crate::skills::hints(false, false),
    })
}

/// What `deploy` did, and what's next: its tools, putting it on a page, and undoing it.
fn deploy_said(app: &str, r: &Value, added: bool) -> String {
    let short = |b: &Value| b.as_str().unwrap_or("").chars().take(12).collect::<String>();
    let mut out = vec![format!("Deployed {app} ({}){}.", short(&r["bundle"]), if r["made"] == true { ", a new app of yours" } else { "" })];
    let tools: Vec<String> = r["tools"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|o| format!("{app}_{o}")).collect();
    match (&r["failed"], r["here"] == true) {
        (Value::String(why), _) => out.push(format!("{why}. Fix it and deploy again.")),
        (_, false) => out.push("Your devices take it within a few seconds.".into()),
        _ if added && tools.is_empty() => out.push("It has no tools.".into()),
        _ if added => out.push(format!("Its tools: {}.", tools.join(", "))),
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

/// The tools it lists: its own, the core's, and every app's.
fn listed(node: &Client) -> Result<Vec<Value>, Value> {
    let apps = node.apps().map_err(|e| json!({ "code": -32000, "message": e.to_string() }))?;
    // MCP Apps' own version of a tool wins.
    let drawn: Vec<Value> = apps::tools().iter().map(|t| t["name"].clone()).collect();
    let core = crate::core_tools::usable(node).iter().flat_map(crate::core_tools::Feature::mcp_tools).filter(|t| !drawn.contains(&t["name"])).collect::<Vec<_>>();
    let mut tools: Vec<Value> = std::iter::once(deploy_tool()).chain(core).collect();
    for app in &apps {
        let name = app["name"].as_str().unwrap_or("");
        // Tools starting with `_` are for the screens.
        for tool in app["tools"].as_array().into_iter().flatten().filter(|tool| !tool["name"].as_str().unwrap_or("").starts_with('_')).filter(|tool| !drawn.contains(&json!(format!("{name}_{}", tool["name"].as_str().unwrap_or(""))))) {
            tools.push(tools::mcp_tool(app, tool, format!("{name}_{}", tool["name"].as_str().unwrap_or(""))));
        }
    }
    Ok(tools)
}

/// A tool's answer, within the budget an agent reads (tools::BUDGET).
fn call_tool(node: &Client, params: &Value, session: &Session) -> Value {
    let mut out = answer_tool(node, params, session);
    let name = params["name"].as_str().unwrap_or("");
    let chars: usize = out["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).map(str::len).sum();
    if chars > tools::BUDGET {
        let tool = match name {
            "search" => Some(code::search_tool(session.mode)["inputSchema"].clone()).map(|input| json!({ "input": input })),
            _ => name.split_once('_').and_then(|(a, o)| tools::find(node, a, o)).map(|(_, o)| o),
        };
        tools::within_budget(&mut out["content"], tool.as_ref());
    }
    out
}

fn answer_tool(node: &Client, params: &Value, session: &Session) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let agent = session.agent.as_deref();
    let input = params["arguments"].as_object().cloned().unwrap_or_default();
    let failed = |text: String| json!({ "isError": true, "content": [{ "type": "text", "text": text }] });
    // This server's own tools, checked as apps' are, in dispatch.
    let own = [code::search_tool(session.mode), code::execute_tool(), deploy_tool()].into_iter().chain(apps::tools()).find(|t| t["name"] == name);
    if let Some(Err(e)) = own.map(|t| tools::check(&t["inputSchema"], &Value::Object(input.clone()))) {
        return failed(format!("{e:#}"));
    }
    if let Some(out) = apps::call(node, name, &Value::Object(input.clone())) {
        return out;
    }
    let with_trace = |mut out: Value, trace: Option<Vec<String>>| {
        if let (Some(lines), Some(content)) = (trace, out["content"].as_array_mut()) {
            content.push(json!({ "type": "text", "text": format!("trace:\n  {}", lines.join("\n  ")) }));
        }
        out
    };
    match name {
        "search" => {
            let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
            match code::catalogs(node) {
                Ok((apps, addable)) => json!({ "content": [{ "type": "text", "text": code::search(&apps, &addable, input.get("query").and_then(Value::as_str).unwrap_or(""), limit, session.mode.reader()) }] }),
                Err(e) => failed(format!("{e:#}")),
            }
        }
        "deploy" => {
            let app = input.get("app").and_then(Value::as_str).unwrap_or("");
            let mut service = String::new();
            let body = match (input.get("files"), input.get("version")) {
                // React screens and a service are built here, on the machine that deploys, as `deploy <folder>` builds them.
                (Some(Value::Object(files)), None) if !node.elsewhere() && (crate::app::deploy::has_screens(files.keys().map(String::as_str)) || files.keys().any(|p| p.starts_with("service/"))) => match crate::build::built_here(node, files) {
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
            match node.post("/v1/deploy", body) {
                Ok(r) => {
                    let added = node.get("/v1/apps").ok().and_then(|a| a.as_array()?.iter().find(|a| a["name"] == app).map(|a| a["added"].as_array().is_some_and(|i| !i.is_empty())));
                    json!({ "content": [{ "type": "text", "text": format!("{}{service}", deploy_said(app, &r, added == Some(true))) }] })
                }
                Err(e) => failed(format!("{e:#}")),
            }
        }
        "execute" => match code::execute(node, input.get("code").and_then(Value::as_str).unwrap_or(""), agent) {
            Ok(result) => json!({ "isError": result["ok"] != true, "content": [{ "type": "text", "text": code::shown(&result) }] }),
            Err(e) => failed(format!("{e:#}")),
        },
        _ => {
            let ran = tools::dispatch(node, name, input, tools::Reader::Mcp, agent, None);
            with_trace(
                match ran.ok {
                    // A script gets an empty list as it is; an agent reading gets it in words.
                    Ok(ok) => match name.split_once('_').filter(|_| ok == json!([])).and_then(|(app, tool)| tools::find(node, app, tool)).and_then(|(a, o)| tools::none_said(node, &a, &o, &ok, tools::Reader::Mcp)) {
                        Some(none) => json!({ "content": [{ "type": "text", "text": none }] }),
                        None => json!({ "content": tools::mcp_content(node, &ok) }),
                    },
                    Err(e) => failed(format!("{e:#}")),
                },
                ran.trace,
            )
        }
    }
}

/// `mcp` on the command line: this server, over stdio.
pub fn commands() -> crate::cli::Command {
    crate::cli::Command {
        help: vec![("mcp [--code]".into(), "the local MCP server, over stdio, for agents: every tool, or with --code, search and execute".into())],
        run: |g| {
            if g.words() != ["mcp"] {
                return Ok(false);
            }
            stdio(g.node.again(), if g.has("code") { Mode::Code } else { Mode::All })?;
            Ok(true)
        },
    }
}
