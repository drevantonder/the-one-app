//! The node hooks this machine's agent harnesses up to itself, on every device: each harness installed here gets this
//! network's MCP server and the node's prompt hook, git sends Repos the node's token, and the built-in skills
//! (`src/skills.rs`) go in each skills folder. A file is written only if it's missing or unchanged since the node last
//! wrote it, so an edit made on disk is never overwritten. The user's own AGENTS.md and skills are an app's, whose
//! service writes them beside these, and marks them as its own.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use serde_json::Value;

use crate::protocol::path::HOOKS_PROMPT;

/// Marks a skill folder as this custom build's to replace, so the nodes of two custom builds on one machine leave each other's alone.
fn marker() -> String {
    format!(".synced-{}", crate::config::CLI)
}

fn built_in(name: &str) -> bool {
    let server = crate::skills::server_name();
    name == server || name.starts_with(&format!("{server}-"))
}

/// How often a node looks again, so a harness installed later gets them too.
const EVERY: Duration = Duration::from_secs(60);

/// Sets up every harness installed on this device, as the node serving it at `addr` from `home` does as it starts and
/// every minute: the MCP server, the prompt hook, git's token for Repos and the built-in skills of the user's apps.
pub async fn keep(node: crate::node::Node, home: PathBuf, addr: String) {
    loop {
        if node.has_network() && let Ok(me) = crate::node::Caller::local(&node) {
            let apps = node.apps(&me).await.unwrap_or_default();
            let (home, addr) = (home.clone(), addr.clone());
            if let Err(err) = tokio::task::spawn_blocking(move || set_up(&home, &addr, &apps)).await {
                tracing::warn!("setting up this machine's agent harnesses: {err}");
            }
        }
        tokio::time::sleep(EVERY).await;
    }
}

fn set_up(node_home: &Path, addr: &str, apps: &[Value]) {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let mut written = Written::load(node_home);
    harnesses(&home, node_home, addr, &mut written);
    let skills: BTreeMap<String, BTreeMap<String, String>> = crate::skills::all(apps).into_iter().map(|s| (s.name.clone(), s.all_files().into_iter().collect())).collect();
    for root in skill_roots(&home) {
        for (name, files) in &skills {
            if let Err(err) = write_skill(&root.join(name), files) {
                tracing::warn!("skill {name}: {err:#}");
            }
        }
        // Built-in skills no longer offered (an app removed) go.
        for dir in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let name = dir.file_name().to_string_lossy().to_string();
            if built_in(&name) && !skills.contains_key(&name) && ours(&dir.path()) {
                let _ = std::fs::remove_dir_all(dir.path());
            }
        }
    }
    written.save();
}

fn skill_roots(home: &Path) -> Vec<PathBuf> {
    let roots = [home.join(".claude/skills"), home.join(".agents/skills")];
    roots.into_iter().filter(|r| r.parent().is_some_and(Path::is_dir) && std::fs::create_dir_all(r).is_ok()).collect()
}

/// Harnesses without settings hooks but with a way to add a message of their own get a plugin that does what the hook
/// does: Pi an extension that says it with each prompt, OpenCode one that keeps it in the session's instructions.
const PI_EXTENSION: &str = r#"// Installed by {{name}}: adds to each prompt what your apps say with it, like
// what other agents have posted since this session last looked.
import { readFileSync } from "node:fs"

const url = "http://{{addr}}{{hook}}"
// Asked as one of this machine's own programs: with the token only its user can read.
const token = () => readFileSync({{token}}, "utf8").trim()

export default function (pi: any) {
  pi.on("before_agent_start", async (event: any, ctx: any) => {
    if (event.messageType !== "user") return
    try {
      const res = await fetch(url, { method: "POST", headers: { connection: "close", "{{header}}": token() }, body: JSON.stringify({ session_id: "pi:" + ctx.sessionManager.getSessionId() }), signal: AbortSignal.timeout(3000) })
      const text = res.ok ? (await res.text()).trim() : ""
      if (text) return { message: { customType: "{{cli}}", content: text, display: true } }
    } catch {}
  })
}
"#;

const OPENCODE_PLUGIN: &str = r#"// Installed by {{name}}: keeps in each session's instructions what your apps say with
// its latest prompt, like what other agents have posted since it last looked.
import { Plugin } from "@opencode/plugin"
import { readFileSync } from "node:fs"

const url = "http://{{addr}}{{hook}}"
// Asked as one of this machine's own programs: with the token only its user can read.
const token = () => readFileSync({{token}}, "utf8").trim()

export default Plugin.define({
  id: "{{cli}}",
  async setup(ctx) {
    await ctx.session.hook("prompt", async (event: any) => {
      try {
        const sid = event.sessionID ?? event.sessionId
        const res = await fetch(url, { method: "POST", headers: { connection: "close", "{{header}}": token() }, body: JSON.stringify({ session_id: "opencode:" + (sid ?? "") }), signal: AbortSignal.timeout(3000) })
        const text = res.ok ? (await res.text()).trim() : ""
        if (sid) {
          if (text) await ctx.session.instructions.entry.put({ sessionID: sid, key: "{{cli}}", value: text })
          else await ctx.session.instructions.entry.remove({ sessionID: sid, key: "{{cli}}" }).catch(() => {})
        } else if (text && event.prompt && typeof event.prompt.text === "string") {
          event.prompt.text += "\n\n" + text
        }
      } catch {}
    })
  },
})
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

/// Gives each harness installed here the node's prompt hook and this network's MCP server, leaving everything else
/// in its settings as it was.
fn harnesses(home: &Path, node_home: &Path, addr: &str, written: &mut Written) {
    let dir = |var: &str, default: PathBuf| std::env::var_os(var).map(PathBuf::from).unwrap_or(default);
    let claude = dir("CLAUDE_CONFIG_DIR", home.join(".claude"));
    let codex = dir("CODEX_HOME", home.join(".codex"));
    let config = dir("XDG_CONFIG_HOME", home.join(".config"));
    let pi = dir("PI_CODING_AGENT_DIR", home.join(".pi/agent"));
    // Asked as one of this machine's own programs: with the token only its user can read.
    let token = node_home.join(crate::api::LOCAL_TOKEN);
    let hook = format!("curl -sf --max-time 3 -H \"{}: $(cat '{}')\" --data-binary @- http://{addr}{} || true", crate::protocol::header::LOCAL_TOKEN, token.display(), crate::protocol::path::HOOKS_PROMPT);
    // Another custom build's node on this machine has a hook of its own, on its own port; an older hook of its own is replaced.
    let port = format!(":{}{}", crate::api::port_of(addr), &HOOKS_PROMPT[..=HOOKS_PROMPT.rfind('/').unwrap_or_default()]);
    let server = crate::skills::server_name();
    // A stable path, so harnesses started without the user's PATH (by launchd, say) still find it after an update; but
    // not another program's that has the CLI's name (an older app's, say): then this binary, which updates in place.
    let installed = home.join(".local/bin").join(crate::config::CLI);
    let exe = std::env::current_exe().and_then(|e| e.canonicalize()).ok();
    let cli = match exe {
        Some(exe) if installed.canonicalize().ok() != Some(exe.clone()) => exe.to_string_lossy().to_string(),
        _ => installed.to_string_lossy().to_string(),
    };
    let token = serde_json::to_string(&token.to_string_lossy()).unwrap_or_default();
    let plugin = |text: &str| {
        let text = text.replace("{{name}}", crate::config::NAME).replace("{{cli}}", crate::config::CLI).replace("{{addr}}", addr).replace("{{hook}}", HOOKS_PROMPT);
        text.replace("{{header}}", crate::protocol::header::LOCAL_TOKEN).replace("{{token}}", &token)
    };
    let s = |v: &str| CstInputValue::String(v.to_string());
    let args = |harness: &str| mcp_args(harness).iter().map(|a| s(a)).collect::<Vec<_>>();
    if claude.is_dir() {
        edit(&claude.join("settings.json"), true, |t| with_hook(t, "UserPromptSubmit", &hook, &port));
        // Claude Code keeps user-scoped MCP servers beside its config folder, or in it if that's moved.
        let file = if std::env::var_os("CLAUDE_CONFIG_DIR").is_some() { claude.join(".claude.json") } else { home.join(".claude.json") };
        let entry = CstInputValue::Object(vec![("type".into(), s("stdio")), ("command".into(), s(&cli)), ("args".into(), CstInputValue::Array(args("claude"))), ("env".into(), CstInputValue::Object(vec![]))]);
        edit(&file, false, |t| with_mcp(t, "mcpServers", &server, entry));
    }
    if codex.is_dir() {
        edit(&codex.join("hooks.json"), true, |t| with_hook(t, "UserPromptSubmit", &hook, &port));
        edit(&codex.join("config.toml"), true, |t| with_codex_mcp(t, &server, &cli, mcp_args("codex")));
    }
    let opencode = config.join("opencode");
    if opencode.is_dir() {
        written.write(&opencode.join("plugins").join(format!("{}-hooks", crate::config::CLI)).join("index.ts"), &plugin(OPENCODE_PLUGIN));
        let entry = CstInputValue::Object(vec![("type".into(), s("local")), ("command".into(), CstInputValue::Array([vec![s(&cli)], args("opencode")].concat())), ("enabled".into(), true.into())]);
        edit(&json_or_jsonc(&opencode, "opencode"), true, |t| with_mcp(t, "mcp", &server, entry));
    }
    let amp = config.join("amp");
    if amp.is_dir() {
        let entry = CstInputValue::Object(vec![("command".into(), s(&cli)), ("args".into(), CstInputValue::Array(args("amp")))]);
        edit(&json_or_jsonc(&amp, "settings"), true, |t| with_mcp(t, "amp.mcpServers", &server, entry));
    }
    if pi.is_dir() {
        written.write(&pi.join("extensions").join(format!("{}-hooks.ts", crate::config::CLI)), &plugin(PI_EXTENSION));
    }
    git(node_home, addr);
}

/// Git reaches Repos at the node (`git clone http://localhost:<port>/repos/<owner>/<repo>.git`) as one of this machine's own
/// programs: the user's git config includes one of the node's, which only they can read, sending its token there and telling
/// the remote helper (src/git_remote.rs) the node's home.
fn git(node_home: &Path, addr: &str) {
    let Some(token) = crate::api::local_token(node_home) else { return };
    let port = crate::api::port_of(addr);
    let file = node_home.join("git-config");
    let hosts: String = ["localhost", "127.0.0.1"].iter().map(|host| format!("[http \"http://{host}:{port}/\"]\n\textraHeader = {}: {token}\n", crate::protocol::header::LOCAL_TOKEN)).collect();
    let text = format!("{hosts}{}", crate::git_wire::home_config(node_home));
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        use std::io::Write;
        let wrote = crate::os::private_file().truncate(true).open(&file).and_then(|mut f| f.write_all(text.as_bytes()));
        if let Err(err) = wrote {
            once(format!("couldn't write {}: {err}", file.display()));
            return;
        }
    }
    let path = file.to_string_lossy().to_string();
    let Ok(out) = std::process::Command::new("git").args(["config", "--global", "--get-all", "include.path"]).output() else { return };
    if !String::from_utf8_lossy(&out.stdout).lines().any(|l| l == path) {
        let _ = std::process::Command::new("git").args(["config", "--global", "--add", "include.path", &path]).status();
    }
}

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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => return once(format!("left {} alone: it couldn't be read ({e})", file.display())),
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

/// Puts `command` among the settings' UserPromptSubmit hooks (Claude Code's and Codex's format), in place of an older
/// hook already there (one whose command has `ours` in it), and says what the text becomes if that changes it.
fn with_hook(text: &str, event: &str, command: &str, ours: &str) -> Result<Option<String>> {
    let root = parse_json(text)?;
    let settings = root.object_value().unwrap();
    let hooks = settings.object_value_or_create("hooks").context("its hooks aren't an object")?;
    let groups = hooks.array_value_or_create(event).with_context(|| format!("its {event} hooks aren't a list"))?;
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
    if dir.exists() && !ours(dir) {
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
    fn load(node_home: &Path) -> Self {
        let path = node_home.join("agents-written.json");
        let hashes = crate::files::read_json(&path);
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
        if now.as_deref() != Some(text) {
            let written = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(file, text));
            if let Err(err) = written {
                once(format!("couldn't write {}: {err}", file.display()));
                return;
            }
        }
        self.hashes.insert(key, hash(text));
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        crate::files::write_json(&self.path, &self.hashes);
    }
}
