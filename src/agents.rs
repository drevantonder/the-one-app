//! Agents: one AGENTS.md and one set of skills, kept the same in every agent harness on every device.
//! The log is the source of truth. A file is written only if it's missing or unchanged since this app
//! last wrote it, so an edit made on disk is never overwritten; `agents_set` keeps it for every device.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::apps::{AppEntry, Me};
use crate::entry::Content;
use crate::native::Reply;

/// Marks a skill folder as this app's to replace. `.theoneapp` is what earlier versions left.
const MARKERS: [&str; 2] = [".synced", ".theoneapp"];

pub fn describe() -> Value {
    let name = json!({ "type": "string", "description": "the skill's folder name, like writing-for-agents" });
    json!({
        "name": "agents", "title": "Agents", "icon": "sparkles",
        "instructions": "agents_get and agents_set read and replace the AGENTS.md every agent harness on every device gets (Codex, Amp, OpenCode). Skills live in every harness's skills folder: agents_skills lists them, agents_skill reads one, agents_skill_set adds or replaces one (all its files), agents_skill_remove removes one.",
        "ops": [
            { "name": "get", "description": "The AGENTS.md every harness on every device gets.", "input": { "type": "object", "properties": {} } },
            { "name": "set", "description": "Replace the AGENTS.md every harness on every device gets.", "input": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" } } } },
            { "name": "skills", "description": "Every skill: its name and description.", "input": { "type": "object", "properties": {} } },
            { "name": "skill", "description": "One skill's files.", "input": { "type": "object", "required": ["name"], "properties": { "name": name } } },
            { "name": "skill_set", "description": "Add or replace a skill on every device: all of its files, SKILL.md first among them.", "input": { "type": "object", "required": ["name", "files"], "properties": { "name": name, "files": { "type": "object", "description": "file path within the skill → its text" } } } },
            { "name": "skill_remove", "description": "Remove a skill from every device.", "input": { "type": "object", "required": ["name"], "properties": { "name": name } } },
        ],
        "widgets": [],
    })
}

/// The state the entries add up to: AGENTS.md, and each skill's files.
fn state(entries: &[AppEntry]) -> (String, BTreeMap<String, BTreeMap<String, String>>) {
    let mut text = String::new();
    let mut skills = BTreeMap::new();
    for e in entries {
        match e.kind.as_str() {
            "agentsmd" => text = e.data["text"].as_str().unwrap_or("").to_string(),
            "skill" => {
                // Checked here, not only in the operation: any member can sign an entry by hand.
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

fn content(kind: &str, data: Value) -> Content {
    Content::new("agents", kind, data)
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

pub fn call(op: String, input: Value, _caller: Me, entries: Vec<AppEntry>) -> Reply {
    Box::pin(async move {
        let (text, skills) = state(&entries);
        let name = || input["name"].as_str().map(str::to_string).context("give the skill's name");
        Ok(match op.as_str() {
            "get" => (json!({ "text": text }), vec![]),
            "set" => {
                let text = input["text"].as_str().context("give the text")?.to_string();
                (json!("AGENTS.md is set; every device's harnesses get it now."), vec![content("agentsmd", json!({ "text": text }))])
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
                (json!(format!("{n} is set on every device.")), vec![content("skill", json!({ "name": n, "files": files }))])
            }
            "skill_remove" => {
                let n = name()?;
                ensure!(skills.contains_key(&n), "no skill {n}");
                (json!(format!("{n} is removed from every device.")), vec![content("skill_removed", json!({ "name": n }))])
            }
            other => bail!("agents has no operation {other}"),
        })
    })
}

fn description(skill_md: &str) -> String {
    skill_md.lines().find_map(|l| l.strip_prefix("description:")).map(|d| d.trim().trim_matches('"').to_string()).unwrap_or_default()
}

/// Writes AGENTS.md and the skills into every harness that's installed on this device.
pub fn sync(entries: &[AppEntry]) {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    if entries.is_empty() {
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let (text, skills) = state(entries);
    let mut written = Written::load(&home);
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
            if !skills.contains_key(&name) && ours(&dir.path()) {
                let _ = std::fs::remove_dir_all(dir.path());
            }
        }
    }
    written.save();
}

fn ours(dir: &Path) -> bool {
    MARKERS.iter().any(|m| dir.join(m).is_file())
}

fn write_skill(dir: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    if dir.exists() && !ours(dir) {
        tracing::info!("left {} alone: it isn't a synced skill", dir.display());
        return Ok(());
    }
    let same = files.iter().all(|(p, t)| std::fs::read_to_string(dir.join(p)).is_ok_and(|now| &now == t)) && dir.join(MARKERS[0]).is_file();
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
    std::fs::write(tmp.join(MARKERS[0]), "This skill is kept the same on every device. Change it with agents_skill_set; delete this file to take the folder over.\n")?;
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
    fn load(home: &Path) -> Self {
        let path = home.join(".local/share").join(crate::brand::CLI).join("agents-written.json");
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
            tracing::info!("left {} alone: it was edited on this device", file.display());
            return;
        }
        if now.as_deref() != Some(text) && std::fs::write(file, text).is_err() {
            return;
        }
        self.hashes.insert(key, hash(text));
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.path, serde_json::to_vec_pretty(&self.hashes).unwrap_or_default());
    }
}
