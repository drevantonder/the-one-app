//! AGENTS.md: one AGENTS.md and one set of skills, kept the same in every agent harness on every device.
//! The log is the source of truth. A file is written only if it's missing or unchanged since this app
//! last wrote it, so an edit made on disk is never overwritten; `agents_set` keeps it for every device.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::apps::Me;
use crate::harnesses::{self, Written};
use crate::services::{Answer, Record, Reply};

/// The built-in service's name, which agents act as when the Vault fills their requests (`vault_fetch`).
pub const NAME: &str = crate::members::AGENTS;


pub fn describe() -> Value {
    let name = json!({ "type": "string", "description": "the skill's folder name, like writing-for-agents" });
    json!({
        "name": "agents", "title": "AGENTS.md", "icon": "brain", "color": "#ff2d55", "description": "Shares your agents' instructions and skills across your devices",
        "instructions": "agents_get and agents_set read and replace the AGENTS.md every agent harness on every device gets (Codex, Amp, OpenCode). Skills live in every harness's skills folder: agents_skills lists them, agents_skill reads one, agents_skill_set adds or replaces one (all its files), agents_skill_remove removes one. agents_history lists AGENTS.md's revisions (or a skill's), and agents_restore brings an old one back. Every device's harnesses also get the board's prompt hook and this install's MCP server.",
        "ops": [
            { "name": "get", "reads": true, "description": "The AGENTS.md every harness on every device gets.", "input": { "type": "object", "properties": {} } },
            { "name": "set", "reads": false, "confirm": true, "description": "Replace the AGENTS.md every harness on every device gets (all of it; history keeps the old one).", "input": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" } } } },
            { "name": "skills", "reads": true, "description": "Every skill: its name and description.", "input": { "type": "object", "properties": {} } },
            { "name": "skill", "reads": true, "description": "One skill's files.", "input": { "type": "object", "required": ["name"], "properties": { "name": name } } },
            { "name": "skill_set", "reads": false, "description": "Add or replace a skill on every device: all of its files, SKILL.md first among them, which starts with frontmatter giving its name and description (what it's for, and when to use it).", "input": { "type": "object", "required": ["name", "files"], "properties": { "name": name, "files": { "type": "object", "description": "file path within the skill → its text" } } } },
            { "name": "skill_remove", "reads": false, "confirm": true, "description": "Remove a skill from every device.", "input": { "type": "object", "required": ["name"], "properties": { "name": name } } },
            { "name": "history", "reads": true, "description": "Every revision of AGENTS.md, or of a skill if given its name, newest first: its id, when, who from which device, its size and what changed.", "input": { "type": "object", "properties": { "name": name } } },
            { "name": "restore", "reads": false, "confirm": true, "description": "Make an older revision (an id from history) the current one again, on every device.", "input": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "description": "the revision's id, from history" } } } },
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
    ensure!(!name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'), "Not added: skill names are lowercase words joined by -");
    ensure!(files.contains_key("SKILL.md"), "Not added: a skill needs a SKILL.md");
    for path in files.keys() {
        ensure!(!path.starts_with('/') && !path.contains('\\') && !path.split('/').any(|p| p.is_empty() || p == "." || p == ".."), "{path} isn't a path inside the skill");
    }
    Ok(())
}

/// Whether a new skill's SKILL.md starts with the frontmatter harnesses load it by: without a name and description,
/// it's never used.
fn introduced(name: &str, files: &BTreeMap<String, String>) -> Result<()> {
    let example = format!("---\nname: {name}\ndescription: what it's for, and when to use it\n---");
    let front = match frontmatter(files.get("SKILL.md").map(String::as_str).unwrap_or("")) {
        Ok(front) => front,
        Err(missing) => bail!("Not added: SKILL.md's {missing}:\n{example}"),
    };
    match front.get("name") {
        None => bail!("Not added: SKILL.md's frontmatter has no name; it starts:\n{example}"),
        Some(n) if n != name => bail!("Not added: SKILL.md's frontmatter names it {n}, not {name}: they must be the same"),
        _ => {}
    }
    ensure!(front.contains_key("description"), "Not added: SKILL.md's frontmatter has no description, which says what it's for and when to use it:\n{example}");
    Ok(())
}

/// A SKILL.md's frontmatter as harnesses read it: `---` on the first line and on one after it, between them each
/// `key: value` with a value, a folded one (`key: >`) on the indented lines after it. Lines may end in \r\n or
/// spaces. Err says what's missing.
fn frontmatter(md: &str) -> std::result::Result<BTreeMap<String, String>, &'static str> {
    let mut lines = md.lines().map(str::trim_end);
    if lines.next() != Some("---") {
        return Err("first line must be ---, starting its frontmatter");
    }
    let rest: Vec<&str> = lines.collect();
    let front = &rest[..rest.iter().position(|l| *l == "---").ok_or("frontmatter must end with a line of ---")?];
    let mut out = BTreeMap::new();
    for (i, line) in front.iter().enumerate() {
        let Some((key, value)) = line.split_once(':').filter(|(k, _)| !k.is_empty() && !k.starts_with([' ', '\t'])) else { continue };
        let mut value = value.trim().to_string();
        if value.starts_with(['>', '|']) && value[1..].chars().all(|c| c == '-' || c == '+' || c.is_ascii_digit()) {
            let folded: Vec<&str> = front[i + 1..].iter().take_while(|l| l.is_empty() || l.starts_with([' ', '\t'])).map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
            value = folded.join(" ");
        }
        let value = value.trim_matches(['"', '\'']).trim();
        if !value.is_empty() {
            out.insert(key.to_string(), value.to_string());
        }
    }
    Ok(out)
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
                ensure!(!harnesses::built_in(&n), "Not added: {n} is a built-in skill's name; choose another");
                introduced(&n, &files)?;
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
    frontmatter(skill_md).ok().and_then(|mut f| f.remove("description")).unwrap_or_default()
}

/// Writes AGENTS.md and the person's skills into every harness installed on this device, after the records change
/// and every minute, so a harness installed later gets them too. The node hooks the harnesses up itself
/// (`src/harnesses.rs`).
pub fn sync(records: &[Record]) {
    // A node given a home of its own (one being tried out, say) writes nowhere else, so it leaves the harnesses alone.
    if std::env::var_os(format!("{}_HOME", crate::config::CLI.to_uppercase())).is_some() {
        return;
    }
    let _turn = harnesses::turn();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let mut written = Written::load();
    let (text, mut skills) = state(records);
    // The built-in skills' names are the node's alone.
    skills.retain(|name, _| !harnesses::built_in(name));
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
    if !text.is_empty() {
        // Agents asked to change their instructions or add a skill otherwise edit this file, which then stays on this
        // device and stops updating.
        let cli = crate::config::CLI;
        let text = format!(
            "<!-- {} keeps this the same on every device: change it with agents_set (`{cli} agents set`), and add a skill for every agent with agents_skill_set (`{cli} agents skill_set`). Edited here, it stays on this device and stops updating. -->\n\n{text}",
            crate::config::NAME
        );
        for dir in [codex, home.join(".config/amp"), home.join(".config/opencode")] {
            if dir.is_dir() {
                written.write(&dir.join("AGENTS.md"), &text);
            }
        }
    }
    for root in harnesses::skill_roots(&home) {
        for (name, files) in &skills {
            if let Err(err) = harnesses::write_skill(&root.join(name), files) {
                tracing::warn!("skill {name}: {err:#}");
            }
        }
        // Skills this app made that are no longer in the log go.
        for dir in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let name = dir.file_name().to_string_lossy().to_string();
            if !harnesses::built_in(&name) && !skills.contains_key(&name) && harnesses::ours(&dir.path()) {
                let _ = std::fs::remove_dir_all(dir.path());
            }
        }
    }
    written.save();
}
