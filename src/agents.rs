//! AGENTS.md: one AGENTS.md and one set of skills, kept the same in every agent harness on every device.
//! The log is the source of truth. A file is written only if it's missing or unchanged since this app
//! last wrote it, so an edit made on disk is never overwritten; `agents_set` keeps it for every device.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail, ensure};
use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use serde_json::{Value, json};

use crate::apps::Me;
use crate::sidecars::{Answer, Record, Reply};

/// The built-in sidecar's name, which agents act as when the Vault fills their requests (`vault_fetch`).
pub const NAME: &str = "agents";

/// Marks a skill folder as this white label's to replace, so the nodes of two white labels on one machine leave each other's alone.
fn marker() -> String {
    format!(".synced-{}", crate::config::CLI)
}

/// What earlier versions marked their folders with: taken over by the next write, never removed.
const OLD_MARKERS: [&str; 2] = [".synced", ".theoneapp"];

pub fn describe() -> Value {
    let name = json!({ "type": "string", "description": "the skill's folder name, like writing-for-agents" });
    json!({
        "name": "agents", "title": "AGENTS.md", "icon": "sparkles", "color": "#ff2d55", "description": "Shares your agents' instructions and skills across your devices",
        "instructions": "agents_get and agents_set read and replace the AGENTS.md every agent harness on every device gets (Codex, Amp, OpenCode). Skills live in every harness's skills folder: agents_skills lists them, agents_skill reads one, agents_skill_set adds or replaces one (all its files), agents_skill_remove removes one. agents_history lists AGENTS.md's revisions (or a skill's), and agents_restore brings an old one back. Every device's harnesses also get the board's prompt hook and this install's MCP server.",
        "ops": [
            { "name": "get", "reads": true, "description": "The AGENTS.md every harness on every device gets.", "input": { "type": "object", "properties": {} } },
            { "name": "set", "confirm": true, "description": "Replace the AGENTS.md every harness on every device gets (all of it; history keeps the old one).", "input": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" } } } },
            { "name": "skills", "reads": true, "description": "Every skill: its name and description.", "input": { "type": "object", "properties": {} } },
            { "name": "skill", "reads": true, "description": "One skill's files.", "input": { "type": "object", "required": ["name"], "properties": { "name": name } } },
            { "name": "skill_set", "description": "Add or replace a skill on every device: all of its files, SKILL.md first among them.", "input": { "type": "object", "required": ["name", "files"], "properties": { "name": name, "files": { "type": "object", "description": "file path within the skill → its text" } } } },
            { "name": "skill_remove", "confirm": true, "description": "Remove a skill from every device.", "input": { "type": "object", "required": ["name"], "properties": { "name": name } } },
            { "name": "history", "reads": true, "description": "Every revision of AGENTS.md, or of a skill if given its name, newest first: its id, when, who from which device, its size and what changed.", "input": { "type": "object", "properties": { "name": name } } },
            { "name": "restore", "confirm": true, "description": "Make an older revision (an id from history) the current one again, on every device.", "input": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "description": "the revision's id, from history" } } } },
        ],
        "widgets": [],
    })
}

/// The state the records add up to: AGENTS.md, and each skill's files.
fn state(records: &[Record]) -> (String, BTreeMap<String, BTreeMap<String, String>>) {
    let mut text = String::new();
    let mut skills = BTreeMap::new();
    for e in records {
        match e.kind.as_str() {
            "agentsmd" => text = e.data["text"].as_str().unwrap_or("").to_string(),
            "skill" => {
                let files: BTreeMap<String, String> = serde_json::from_value(e.data["files"].clone()).unwrap_or_default();
                match e.data["name"].as_str() {
                    Some(name) if valid(name, &files).is_ok() => {
                        skills.insert(name.to_string(), files);
                    }
                    _ => tracing::warn!("ignored a skill with a bad name or path"),
                }
            }
            "skill_removed" => {
                if let Some(name) = e.data["name"].as_str() {
                    skills.remove(name);
                }
            }
            _ => {}
        }
    }
    (text, skills)
}


/// A skill's name is one folder, and its files stay inside it.
fn valid(name: &str, files: &BTreeMap<String, String>) -> Result<()> {
    ensure!(!name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'), "skill names are lowercase words joined by -");
    ensure!(files.contains_key("SKILL.md"), "a skill needs a SKILL.md");
    for path in files.keys() {
        ensure!(!path.starts_with('/') && !path.contains('\\') && !path.split('/').any(|p| p.is_empty() || p == "." || p == ".."), "{path} isn't a path inside the skill");
    }
    Ok(())
}

pub fn call(op: String, input: Value, _caller: Me, records: Vec<Record>) -> Reply {
    Box::pin(async move {
        let (text, skills) = state(&records);
        let name = || input["name"].as_str().map(str::to_string).context("give the skill's name");
        let (ok, kept) = match op.as_str() {
            "get" => (json!({ "text": text }), vec![]),
            "set" => {
                let text = input["text"].as_str().context("give the text")?.to_string();
                (json!("AGENTS.md is set; every device's harnesses get it now."), vec![("agentsmd".to_string(), json!({ "text": text }))])
            }
            "skills" => {
                let list: Vec<Value> = skills.iter().map(|(n, files)| json!({ "name": n, "description": description(files.get("SKILL.md").map(String::as_str).unwrap_or("")) })).collect();
                (json!(list), vec![])
            }
            "skill" => {
                let n = name()?;
                (json!({ "name": n, "files": skills.get(&n).with_context(|| format!("no skill {n}"))? }), vec![])
            }
            "skill_set" => {
                let n = name()?;
                let files: BTreeMap<String, String> = serde_json::from_value(input["files"].clone()).context("files is path → text")?;
                valid(&n, &files)?;
                (json!(format!("{n} is set on every device.")), vec![("skill".to_string(), json!({ "name": n, "files": files }))])
            }
            "skill_remove" => {
                let n = name()?;
                ensure!(skills.contains_key(&n), "no skill {n}");
                (json!(format!("{n} is removed from every device.")), vec![("skill_removed".to_string(), json!({ "name": n }))])
            }
            "history" => (json!(history(&records, input["name"].as_str())), vec![]),
            "restore" => {
                let id = input["id"].as_str().context("give the revision's id, from history")?;
                let e = records.iter().find(|e| e.id == id).with_context(|| format!("no revision {id}"))?;
                match e.kind.as_str() {
                    "agentsmd" => (json!("That AGENTS.md is current again on every device."), vec![("agentsmd".to_string(), e.data.clone())]),
                    "skill" => {
                        let n = e.data["name"].as_str().unwrap_or_default();
                        let files: BTreeMap<String, String> = serde_json::from_value(e.data["files"].clone()).unwrap_or_default();
                        valid(n, &files)?;
                        (json!(format!("That {n} is current again on every device.")), vec![("skill".to_string(), e.data.clone())])
                    }
                    _ => bail!("{id} isn't a revision of AGENTS.md or a skill"),
                }
            }
            other => bail!("agents has no operation {other}"),
        };
        Ok(Answer { records: kept, ..Answer::new(ok) })
    })
}

/// AGENTS.md's revisions, or a skill's, newest first, each with what changed since the one before.
fn history(records: &[Record], skill: Option<&str>) -> Vec<Value> {
    let mut before: Option<String> = None;
    let mut out = Vec::new();
    for e in records {
        let (now, change) = match (skill, e.kind.as_str()) {
            (None, "agentsmd") => {
                let text = e.data["text"].as_str().unwrap_or_default().to_string();
                let change = first_change(before.as_deref().unwrap_or_default(), &text);
                (Some(text), change)
            }
            (Some(n), "skill") if e.data["name"] == n => {
                let files: BTreeMap<String, String> = serde_json::from_value(e.data["files"].clone()).unwrap_or_default();
                let text = files.iter().map(|(p, t)| format!("--- {p}\n{t}\n")).collect::<String>();
                let change = first_change(before.as_deref().unwrap_or_default(), &text);
                (Some(text), change)
            }
            (Some(n), "skill_removed") if e.data["name"] == n => (None, "removed".to_string()),
            _ => continue,
        };
        out.push(json!({ "id": e.id, "time": e.time, "by": e.by, "device": e.device, "size": now.as_ref().map_or(0, String::len), "change": change }));
        before = now;
    }
    out.reverse();
    out
}

/// The first line of `now` that isn't the same in `before`, or what was cut.
fn first_change(before: &str, now: &str) -> String {
    let (mut old, mut new) = (before.lines(), now.lines());
    loop {
        match (old.next(), new.next()) {
            (Some(a), Some(b)) if a == b => continue,
            (_, Some(b)) => return format!("+ {}", b.chars().take(100).collect::<String>()),
            (Some(a), None) => return format!("- {}", a.chars().take(100).collect::<String>()),
            (None, None) => return "no change".to_string(),
        }
    }
}

fn description(skill_md: &str) -> String {
    skill_md.lines().find_map(|l| l.strip_prefix("description:")).map(|d| d.trim().trim_matches('"').to_string()).unwrap_or_default()
}

/// Sets up every harness installed on this device: AGENTS.md, the skills, the board's prompt hook and the MCP server.
/// It looks again every minute, so a harness installed later gets them too.
pub fn sync(records: &[Record]) {
    // A node given a home of its own (one being tried out, say) writes nowhere else, so it leaves the harnesses alone.
    if std::env::var_os(format!("{}_HOME", crate::config::CLI.to_uppercase())).is_some() {
        return;
    }
    static LATEST: Mutex<Vec<Record>> = Mutex::new(Vec::new());
    static AGAIN: std::sync::Once = std::sync::Once::new();
    *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = records.to_vec();
    AGAIN.call_once(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                // The person's apps, for their skills: asked here, where waiting for an answer holds nothing up.
                let apps = ureq::get(&format!("http://{}/v1/apps", crate::api::addr())).call().ok().and_then(|mut r| r.body_mut().read_json::<Value>().ok());
                if let Some(apps) = apps.and_then(|a| a.as_array().cloned()) {
                    *APPS.lock().unwrap_or_else(|e| e.into_inner()) = Some(apps);
                }
                let records = LATEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
                set_up(&records);
            }
        });
    });
    set_up(records);
}

/// The apps installed for this server's person, as the local API last listed them.
static APPS: Mutex<Option<Vec<Value>>> = Mutex::new(None);

fn set_up(records: &[Record]) {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let mut written = Written::load();
    harnesses(&home, &mut written);
    let (text, mut skills) = state(records);
    // The built-in skills (src/skills.rs) go beside the person's own, which win a name they share. Until the apps are
    // known, those already written stay.
    let apps = APPS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let built_in = crate::skills::all(apps.as_deref().unwrap_or_default());
    for skill in &built_in {
        skills.entry(skill.name.clone()).or_insert_with(|| skill.all_files().into_iter().collect());
    }
    let keep = |name: &str| skills.contains_key(name) || (apps.is_none() && name.starts_with(&format!("{}-", crate::skills::server_name())));
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
    if !text.is_empty() {
        for dir in [codex, home.join(".config/amp"), home.join(".config/opencode")] {
            if dir.is_dir() {
                written.write(&dir.join("AGENTS.md"), &text);
            }
        }
    }
    for root in [home.join(".claude/skills"), home.join(".agents/skills")] {
        if !root.parent().is_some_and(Path::is_dir) {
            continue;
        }
        let _ = std::fs::create_dir_all(&root);
        for (name, files) in &skills {
            if let Err(err) = write_skill(&root.join(name), files) {
                tracing::warn!("skill {name}: {err:#}");
            }
        }
        // Skills this app made that are no longer in the log go.
        for dir in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let name = dir.file_name().to_string_lossy().to_string();
            if !keep(&name) && ours(&dir.path()) {
                let _ = std::fs::remove_dir_all(dir.path());
            }
        }
    }
    written.save();
}

/// Where the board answers a harness's prompt hook with what other agents posted since that session last looked.
const HOOK_PATH: &str = "/hooks/board/news";

/// Harnesses without settings hooks but with a way to add a message of their own get an extension that does what the
/// hook does. OpenCode gets none: it could only add to the person's own message, where they see it; the board's
/// instructions reach it in AGENTS.md and the MCP server's.
const PI_EXTENSION: &str = r#"// Installed by {{name}}'s Board: adds to each prompt what other
// agents have posted since this session last looked.
const url = "http://{{addr}}/hooks/board/news"

export default function (pi: any) {
  pi.on("before_agent_start", async (event: any, ctx: any) => {
    if (event.messageType !== "user") return
    try {
      const res = await fetch(url, { method: "POST", headers: { connection: "close" }, body: JSON.stringify({ session_id: "pi:" + ctx.sessionManager.getSessionId() }), signal: AbortSignal.timeout(3000) })
      const text = res.ok ? (await res.text()).trim() : ""
      if (text) return { message: { customType: "{{cli}}-board", content: text, display: true } }
    } catch {}
  })
}
"#;

/// The MCP entry each harness gets when it has none: `<cli> mcp`, every tool, for those that find tools themselves or
/// script them (Claude Code and Codex search deferred tools, OpenCode wraps them in its own code mode); `<cli> mcp
/// --code` for Amp, which loads every tool up front. As checked on 2026-11-01; check again after their big releases.
fn mcp_args(harness: &str) -> &'static [&'static str] {
    match harness {
        "amp" => &["mcp", "--code"],
        _ => &["mcp"],
    }
}

/// Gives each harness installed here the board's prompt hook and this install's MCP server, leaving everything else
/// in its settings as it was.
fn harnesses(home: &Path, written: &mut Written) {
    let dir = |var: &str, default: PathBuf| std::env::var_os(var).map(PathBuf::from).unwrap_or(default);
    let claude = dir("CLAUDE_CONFIG_DIR", home.join(".claude"));
    let codex = dir("CODEX_HOME", home.join(".codex"));
    let config = dir("XDG_CONFIG_HOME", home.join(".config"));
    let pi = dir("PI_CODING_AGENT_DIR", home.join(".pi/agent"));
    let addr = crate::api::addr();
    let hook = format!("curl -sf --max-time 3 --data-binary @- http://{addr}{HOOK_PATH} || true");
    // Another white label's node on this machine has a hook of its own, on its own port.
    let port = format!(":{}{HOOK_PATH}", addr.rsplit(':').next().unwrap_or_default());
    let server = crate::skills::server_name();
    // A stable path, so harnesses started without the user's PATH (by launchd, say) still find it after an update; but
    // not another program's that has the CLI's name (an older app's, say): then this binary, which updates in place.
    let installed = home.join(".local/bin").join(crate::config::CLI);
    let exe = std::env::current_exe().and_then(|e| e.canonicalize()).ok();
    let cli = match exe {
        Some(exe) if installed.canonicalize().ok() != Some(exe.clone()) => exe.to_string_lossy().to_string(),
        _ => installed.to_string_lossy().to_string(),
    };
    let plugin = |text: &str| text.replace("{{name}}", crate::config::NAME).replace("{{cli}}", crate::config::CLI).replace("{{addr}}", &addr);
    let s = |v: &str| CstInputValue::String(v.to_string());
    let args = |harness: &str| mcp_args(harness).iter().map(|a| s(a)).collect::<Vec<_>>();
    if claude.is_dir() {
        edit(&claude.join("settings.json"), true, |t| with_hook(t, &hook, &port));
        // Claude Code keeps user-scoped MCP servers beside its config folder, or in it if that's moved.
        let file = if std::env::var_os("CLAUDE_CONFIG_DIR").is_some() { claude.join(".claude.json") } else { home.join(".claude.json") };
        let entry = CstInputValue::Object(vec![("type".into(), s("stdio")), ("command".into(), s(&cli)), ("args".into(), CstInputValue::Array(args("claude"))), ("env".into(), CstInputValue::Object(vec![]))]);
        edit(&file, false, |t| with_mcp(t, "mcpServers", &server, entry));
    }
    if codex.is_dir() {
        edit(&codex.join("hooks.json"), true, |t| with_hook(t, &hook, &port));
        edit(&codex.join("config.toml"), true, |t| with_codex_mcp(t, &server, &cli, mcp_args("codex")));
    }
    let opencode = config.join("opencode");
    if opencode.is_dir() {
        written.remove(&opencode.join("plugins").join(format!("{}-board.js", crate::config::CLI)));
        let entry = CstInputValue::Object(vec![("type".into(), s("local")), ("command".into(), CstInputValue::Array([vec![s(&cli)], args("opencode")].concat())), ("enabled".into(), true.into())]);
        edit(&json_or_jsonc(&opencode, "opencode"), true, |t| with_mcp(t, "mcp", &server, entry));
    }
    let amp = config.join("amp");
    if amp.is_dir() {
        let entry = CstInputValue::Object(vec![("command".into(), s(&cli)), ("args".into(), CstInputValue::Array(args("amp")))]);
        edit(&json_or_jsonc(&amp, "settings"), true, |t| with_mcp(t, "amp.mcpServers", &server, entry));
    }
    if pi.is_dir() {
        written.write(&pi.join("extensions").join(format!("{}-board.ts", crate::config::CLI)), &plugin(PI_EXTENSION));
    }
}

/// `<name>.jsonc` if the harness has one, else `<name>.json`.
fn json_or_jsonc(dir: &Path, name: &str) -> PathBuf {
    let jsonc = dir.join(format!("{name}.jsonc"));
    if jsonc.is_file() { jsonc } else { dir.join(format!("{name}.json")) }
}

/// Changes a harness's settings file to what `change` makes of its text (`None`: nothing to change). A file it can't
/// read or parse is left as it is; one that doesn't exist is made only if `create`.
fn edit(file: &Path, create: bool, change: impl FnOnce(&str) -> Result<Option<String>>) {
    // A settings file kept in a dotfiles repo is often a link: change what it links to, and keep the link.
    let file = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => String::new(),
        Err(_) => return,
    };
    let result = change(&text).and_then(|new| match new {
        Some(new) => replace(&file, &new).map(|()| true),
        None => Ok(false),
    });
    match result {
        Ok(true) => tracing::info!("set up {}", file.display()),
        Ok(false) => {}
        Err(err) => once(format!("left {} alone: {err:#}", file.display())),
    }
}

/// Says why something was left alone once, not every minute.
fn once(said: String) {
    static TOLD: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut told = TOLD.lock().unwrap_or_else(|e| e.into_inner());
    if !told.contains(&said) {
        tracing::info!("{said}");
        told.push(said);
    }
}

/// Writes `text` over `file` all at once, so a harness reading it never sees half, keeping its permissions.
fn replace(file: &Path, text: &str) -> Result<()> {
    let name = file.file_name().context("no file name")?.to_string_lossy();
    let tmp = file.with_file_name(format!(".{name}.{}-tmp", crate::config::CLI));
    std::fs::write(&tmp, text)?;
    if let Ok(meta) = std::fs::metadata(file) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, file)?;
    Ok(())
}

fn parse_json(text: &str) -> Result<CstRootNode> {
    let text = if text.trim().is_empty() { "{}\n" } else { text };
    let root = CstRootNode::parse(text, &ParseOptions::default()).map_err(|e| anyhow::anyhow!("can't read it: {e}"))?;
    ensure!(root.object_value().is_some(), "it isn't a JSON object");
    Ok(root)
}

/// Puts `command` among the settings' UserPromptSubmit hooks (Claude Code's and Codex's format), in place of a board
/// hook already there (one whose command has `ours` in it), and says what the text becomes if that changes it.
fn with_hook(text: &str, command: &str, ours: &str) -> Result<Option<String>> {
    let root = parse_json(text)?;
    let settings = root.object_value().unwrap();
    let hooks = settings.object_value_or_create("hooks").context("its hooks aren't an object")?;
    let groups = hooks.array_value_or_create("UserPromptSubmit").context("its UserPromptSubmit hooks aren't a list")?;
    let (mut found, mut changed) = (false, false);
    for group in groups.elements() {
        let Some(list) = group.as_object().and_then(|g| g.array_value("hooks")) else { continue };
        let mut emptied = false;
        for hook in list.elements() {
            let Some(prop) = hook.as_object().and_then(|h| h.get("command")) else { continue };
            let Some(now) = prop.value().and_then(|v| v.as_string_lit()).and_then(|s| s.decoded_value().ok()) else { continue };
            if !now.contains(ours) {
                continue;
            }
            if found {
                hook.remove();
                emptied = true;
            } else if now != command {
                prop.set_value(command.into());
            }
            changed |= found || now != command;
            found = true;
        }
        if emptied && list.elements().is_empty() {
            group.remove();
        }
    }
    if !found {
        let hook = CstInputValue::Object(vec![("type".into(), "command".into()), ("command".into(), command.into())]);
        groups.append(CstInputValue::Object(vec![("hooks".into(), CstInputValue::Array(vec![hook]))]));
        changed = true;
    }
    Ok(changed.then(|| root.to_string()))
}

/// Adds `entry` as MCP server `name` in the object at top-level key `key`, unless there's one by that name already.
fn with_mcp(text: &str, key: &str, name: &str, entry: CstInputValue) -> Result<Option<String>> {
    let root = parse_json(text)?;
    let servers = root.object_value().unwrap().object_value_or_create(key).with_context(|| format!("its {key} isn't an object"))?;
    if servers.get(name).is_some() {
        return Ok(None);
    }
    servers.append(name, entry);
    Ok(Some(root.to_string()))
}

/// Adds `[mcp_servers.<name>]` to Codex's config.toml, unless there's one by that name already.
fn with_codex_mcp(text: &str, name: &str, cli: &str, args: &[&str]) -> Result<Option<String>> {
    let mut doc: toml_edit::DocumentMut = text.parse().context("can't read it")?;
    if !doc.contains_key("mcp_servers") {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        doc.insert("mcp_servers", toml_edit::Item::Table(table));
    }
    let mut server = toml_edit::Table::new();
    server["command"] = toml_edit::value(cli);
    server["args"] = toml_edit::value(toml_edit::Array::from_iter(args.iter().copied()));
    match &mut doc["mcp_servers"] {
        toml_edit::Item::Table(servers) if !servers.contains_key(name) => {
            servers.insert(name, toml_edit::Item::Table(server));
        }
        toml_edit::Item::Value(toml_edit::Value::InlineTable(servers)) if !servers.contains_key(name) => {
            servers.insert(name, toml_edit::Value::InlineTable(server.into_inline_table()));
        }
        toml_edit::Item::Table(_) | toml_edit::Item::Value(toml_edit::Value::InlineTable(_)) => return Ok(None),
        _ => bail!("its mcp_servers isn't a table"),
    };
    Ok(Some(doc.to_string()))
}

fn ours(dir: &Path) -> bool {
    dir.join(marker()).is_file()
}

fn write_skill(dir: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    if dir.exists() && !ours(dir) && !OLD_MARKERS.iter().any(|m| dir.join(m).is_file()) {
        once(format!("left {} alone: it isn't a synced skill", dir.display()));
        return Ok(());
    }
    let same = files.iter().all(|(p, t)| std::fs::read_to_string(dir.join(p)).is_ok_and(|now| &now == t)) && ours(dir);
    if same {
        return Ok(());
    }
    let tmp = dir.with_extension("syncing");
    let _ = std::fs::remove_dir_all(&tmp);
    for (path, text) in files {
        let file = tmp.join(path);
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(&file, text)?;
    }
    std::fs::write(tmp.join(marker()), "This skill is kept the same on every device. Change it with agents_skill_set; delete this file to take the folder over.\n")?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::rename(&tmp, dir)?;
    Ok(())
}

/// What this app last wrote to each file, so it never overwrites an edit made on disk.
struct Written {
    path: PathBuf,
    hashes: HashMap<String, String>,
}

impl Written {
    fn load() -> Self {
        let path = crate::home().join("agents-written.json");
        let hashes = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Written { path, hashes }
    }

    fn write(&mut self, file: &Path, text: &str) {
        let key = file.to_string_lossy().to_string();
        let now = std::fs::read_to_string(file).ok();
        let hash = |s: &str| blake3::hash(s.as_bytes()).to_hex().to_string();
        let untouched = match &now {
            None => true,
            Some(now) => now == text || self.hashes.get(&key).is_some_and(|h| *h == hash(now)),
        };
        if !untouched {
            once(format!("left {} alone: it was edited on this device", file.display()));
            return;
        }
        if now.as_deref() != Some(text) && file.parent().is_none_or(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(file, text).is_err() {
            return;
        }
        self.hashes.insert(key, hash(text));
    }

    /// Deletes a file this app wrote and no longer writes, unless it was edited on this device.
    fn remove(&mut self, file: &Path) {
        let key = file.to_string_lossy().to_string();
        let Some(hash) = self.hashes.remove(&key) else { return };
        if std::fs::read(file).is_ok_and(|now| blake3::hash(&now).to_hex().to_string() == hash) {
            let _ = std::fs::remove_file(file);
        }
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.path, serde_json::to_vec_pretty(&self.hashes).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOOK: &str = "curl -sf --max-time 3 --data-binary @- http://127.0.0.1:4747/hooks/board/news || true";
    const OURS: &str = ":4747/hooks/board/news";

    fn hooks(text: &str) -> Vec<String> {
        let v: Value = serde_json::from_str(text).unwrap();
        v["hooks"]["UserPromptSubmit"].as_array().unwrap().iter().flat_map(|g| g["hooks"].as_array().unwrap().iter().map(|h| h["command"].as_str().unwrap().to_string())).collect()
    }

    #[test]
    fn hook_goes_into_new_and_empty_settings() {
        for text in ["", "{}", "{\n}\n"] {
            let out = with_hook(text, HOOK, OURS).unwrap().unwrap();
            assert_eq!(hooks(&out), [HOOK]);
            assert_eq!(with_hook(&out, HOOK, OURS).unwrap(), None, "a second run changes nothing");
        }
    }

    #[test]
    fn hook_keeps_everything_else_exactly() {
        let text = "{\n  // mine\n  \"model\": \"opus\",\n  \"hooks\": {\n    \"Stop\": [{ \"hooks\": [{ \"type\": \"command\", \"command\": \"say done\" }] }],\n    \"UserPromptSubmit\": [\n      { \"matcher\": \"\", \"hooks\": [{ \"type\": \"command\", \"command\": \"echo hi\" }] }\n    ]\n  },\n  \"zed\": 1\n}\n";
        let out = with_hook(text, HOOK, OURS).unwrap().unwrap();
        assert!(out.starts_with("{\n  // mine\n  \"model\": \"opus\",\n  \"hooks\": {\n    \"Stop\": [{ \"hooks\": [{ \"type\": \"command\", \"command\": \"say done\" }] }],\n    \"UserPromptSubmit\": [\n      { \"matcher\": \"\", \"hooks\": [{ \"type\": \"command\", \"command\": \"echo hi\" }] },"), "{out}");
        assert!(out.ends_with("  \"zed\": 1\n}\n"), "{out}");
        let plain = out.replace("  // mine\n", "");
        assert_eq!(hooks(&plain), ["echo hi", HOOK]);
    }

    #[test]
    fn the_same_hook_is_left_alone_and_an_old_one_swapped() {
        // What the old One App wrote: the same command, so nothing to do.
        let old_app = format!("{{\n  \"hooks\": {{\n    \"UserPromptSubmit\": [\n      {{\n        \"hooks\": [\n          {{\n            \"command\": \"{HOOK}\",\n            \"type\": \"command\"\n          }}\n        ]\n      }}\n    ]\n  }}\n}}\n");
        assert_eq!(with_hook(&old_app, HOOK, OURS).unwrap(), None);
        // An older command for the same node is replaced where it is, and a duplicate goes.
        let old = "curl -s http://127.0.0.1:4747/hooks/board/news";
        let text = format!("{{\"hooks\":{{\"UserPromptSubmit\":[{{\"hooks\":[{{\"type\":\"command\",\"command\":\"{old}\"}},{{\"type\":\"command\",\"command\":\"echo hi\"}}]}},{{\"hooks\":[{{\"type\":\"command\",\"command\":\"{HOOK}\"}}]}}]}}}}");
        let out = with_hook(&text, HOOK, OURS).unwrap().unwrap();
        assert_eq!(hooks(&out), [HOOK, "echo hi"]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1, "the emptied group goes: {out}");
        // Another white label's board hook, on another port, stays.
        let other = "curl -sf --max-time 3 --data-binary @- http://127.0.0.1:4848/hooks/board/news || true";
        let text = format!("{{\"hooks\":{{\"UserPromptSubmit\":[{{\"hooks\":[{{\"type\":\"command\",\"command\":\"{other}\"}}]}}]}}}}");
        assert_eq!(hooks(&with_hook(&text, HOOK, OURS).unwrap().unwrap()), [other, HOOK]);
    }

    #[test]
    fn settings_it_cant_read_are_never_rewritten() {
        for text in ["{ \"model\": ", "[1, 2]", "{\"hooks\": []}", "{\"hooks\": {\"UserPromptSubmit\": {}}}"] {
            assert!(with_hook(text, HOOK, OURS).is_err(), "{text}");
        }
        assert!(with_mcp("{\"mcp\": 3}", "mcp", "the-one-app", "x".into()).is_err());
        assert!(with_codex_mcp("model = ", "the-one-app", "one", &["mcp"]).is_err());
        assert!(with_codex_mcp("mcp_servers = 3\n", "the-one-app", "one", &["mcp"]).is_err());
    }

    #[test]
    fn harnesses_that_load_every_tool_get_code_mode() {
        assert_eq!(mcp_args("amp"), &["mcp", "--code"]);
        for h in ["claude", "codex", "opencode"] {
            assert_eq!(mcp_args(h), &["mcp"]);
        }
    }

    #[test]
    fn mcp_server_is_added_once_and_never_replaced() {
        let entry = || CstInputValue::Object(vec![("command".into(), "/home/a/.local/bin/one".into()), ("args".into(), CstInputValue::Array(vec!["mcp".into()]))]);
        let text = "{\n  \"amp.mcpServers\": {\n    \"paper\": { \"command\": \"paper\" }\n  },\n  \"amp.theme\": \"dark\"\n}\n";
        let out = with_mcp(text, "amp.mcpServers", "the-one-app", entry()).unwrap().unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["amp.mcpServers"]["paper"]["command"], "paper");
        assert_eq!(v["amp.mcpServers"]["the-one-app"]["args"], json!(["mcp"]));
        assert!(out.starts_with("{\n  \"amp.mcpServers\": {\n    \"paper\": { \"command\": \"paper\" },\n"), "{out}");
        assert!(out.ends_with("  \"amp.theme\": \"dark\"\n}\n"), "{out}");
        assert_eq!(with_mcp(&out, "amp.mcpServers", "the-one-app", entry()).unwrap(), None);
        // One the user changed stays as they left it.
        let theirs = "{\"mcpServers\": {\"the-one-app\": {\"command\": \"one\", \"args\": [\"mcp\"]}}}";
        assert_eq!(with_mcp(theirs, "mcpServers", "the-one-app", entry()).unwrap(), None);
        let out = with_mcp("", "mcp", "the-one-app", entry()).unwrap().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["mcp"]["the-one-app"]["command"], "/home/a/.local/bin/one");
    }

    #[test]
    fn codex_config_keeps_its_comments_and_tables() {
        let text = "# my config\nmodel = \"gpt-5.5\" # best\n\n[mcp_servers.nuxt_ui]\nurl = \"https://ui.nuxt.com/mcp\"\n\n[features]\ngoals = true\n";
        let out = with_codex_mcp(text, "the-one-app", "/home/a/.local/bin/one", &["mcp"]).unwrap().unwrap();
        assert!(out.starts_with(text.trim_end_matches("[features]\ngoals = true\n")) || out.starts_with(text), "{out}");
        assert!(out.contains("# my config\nmodel = \"gpt-5.5\" # best\n"), "{out}");
        assert!(out.contains("[features]\ngoals = true\n"), "{out}");
        assert!(out.contains("[mcp_servers.the-one-app]\ncommand = \"/home/a/.local/bin/one\"\nargs = [\"mcp\"]\n"), "{out}");
        assert!(!out.contains("[mcp_servers]\n"), "{out}");
        assert_eq!(with_codex_mcp(&out, "the-one-app", "one", &["mcp"]).unwrap(), None);
        let out = with_codex_mcp("", "the-one-app", "one", &["mcp"]).unwrap().unwrap();
        assert_eq!(out, "[mcp_servers.the-one-app]\ncommand = \"one\"\nargs = [\"mcp\"]\n");
        let inline = "mcp_servers = { paper = { command = \"paper\" } }\n";
        let out = with_codex_mcp(inline, "the-one-app", "one", &["mcp"]).unwrap().unwrap();
        assert!(out.contains("paper = { command = \"paper\" }") && out.contains("the-one-app"), "{out}");
    }

    #[test]
    fn the_server_is_named_after_the_white_label() {
        let name = crate::skills::server_name();
        assert!(!name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !name.starts_with('-') && !name.ends_with('-'), "{name}");
    }

    fn record(id: &str, kind: &str, data: Value) -> Record {
        Record { id: id.into(), kind: kind.into(), data, time: id.len() as u64, by: "Andre".into(), device: "mac".into() }
    }

    #[test]
    fn history_lists_revisions_newest_first() {
        let skill = |t: &str| json!({ "name": "s", "files": { "SKILL.md": t } });
        let records = vec![
            record("a", "agentsmd", json!({ "text": "one\ntwo\n" })),
            record("b", "skill", skill("x")),
            record("c", "agentsmd", json!({ "text": "one\nthree\n" })),
            record("d", "skill_removed", json!({ "name": "s" })),
            record("e", "skill", skill("y")),
        ];
        let md = history(&records, None);
        assert_eq!(md.iter().map(|r| r["id"].as_str().unwrap()).collect::<Vec<_>>(), ["c", "a"]);
        assert_eq!(md[0]["change"], "+ three");
        assert_eq!(md[1]["change"], "+ one");
        assert_eq!(md[0]["size"], 10);
        let s = history(&records, Some("s"));
        assert_eq!(s.iter().map(|r| r["change"].as_str().unwrap()).collect::<Vec<_>>(), ["+ --- SKILL.md", "removed", "+ --- SKILL.md"]);
        assert_eq!(first_change("a\nb\n", "a\n"), "- b");
        assert_eq!(first_change("a\n", "a\n"), "no change");
    }
}
