//! AGENTS.md's service: writes the user's AGENTS.md and skills, as its app keeps them, into every agent harness on its
//! device, as it starts and every minute after, so a harness installed later gets them too. A file is written only if
//! it's missing or unchanged since this service last wrote it, so an edit made on disk is never overwritten;
//! `agents_set` keeps it for every device. The node writes the rest (its MCP server, prompt hook and built-in skills).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use one_service::{Answer, Call, Host, Service};
use serde_json::json;

/// The custom build's command (`one`), and whether this service writes into the harnesses: not for a node with a
/// home of its own (a throwaway network's), which writes nowhere else.
static HERE: OnceLock<(String, bool)> = OnceLock::new();

fn main() -> Result<()> {
    let cli = std::env::var("ONE_CLI").unwrap_or_else(|_| "one".into());
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let default = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join(&cli).join("agents");
    let writes = one_service::dir("agents") == default;
    if !writes {
        eprintln!("agents: its node has a home of its own, so it leaves this machine's harnesses alone");
    }
    let _ = HERE.set((cli, writes));
    one_service::serve(Service { name: "agents", prefer_here: &[], call, start: Some(start) })
}

fn call(_: &Host, call: &Call) -> Result<Answer> {
    bail!("agents has no function {}", call.function)
}

fn start(host: &Host) {
    let host = host.clone();
    std::thread::spawn(move || {
        loop {
            if HERE.get().is_some_and(|(_, writes)| *writes)
                && let Err(err) = sync(&host)
            {
                once(format!("{err:#}"));
            }
            std::thread::sleep(Duration::from_secs(60));
        }
    });
}

/// Marks a skill folder as this custom build's AGENTS.md's, so the node's built-in skills and another custom build's
/// are left alone.
fn marker() -> String {
    format!(".synced-{}-agents", cli())
}

/// What the node marked its skills with: taken over by the next write, never removed.
fn node_marker() -> String {
    format!(".synced-{}", cli())
}

fn cli() -> &'static str {
    HERE.get().map_or("one", |(cli, _)| cli.as_str())
}

fn sync(host: &Host) -> Result<()> {
    let files = host.post("/api/call", &json!({ "app": "agents", "tool": "_files", "input": {} }))?;
    let files = &files["ok"];
    let text = files["text"].as_str().context("its app answered with no text")?;
    let skills: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_value(files["skills"].clone()).context("its app's skills")?;
    let home = std::env::var_os("HOME").map(PathBuf::from).context("no HOME")?;
    let mut written = Written::load(host);
    if !text.is_empty() {
        let cli = cli();
        let text = format!(
            "<!-- Your AGENTS.md app keeps this the same on every device: change it with agents_set (`{cli} agents set`), and add a skill for every agent with agents_skill_set (`{cli} agents skill_set`). Edited here, it stays on this device and stops updating. -->\n\n{text}"
        );
        let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
        for dir in [codex, home.join(".config/amp"), home.join(".config/opencode")] {
            if dir.is_dir() {
                written.write(&dir.join("AGENTS.md"), &text);
            }
        }
    }
    for root in [home.join(".claude/skills"), home.join(".agents/skills")] {
        if !root.parent().is_some_and(Path::is_dir) || std::fs::create_dir_all(&root).is_err() {
            continue;
        }
        for (name, files) in &skills {
            if let Err(err) = write_skill(&root.join(name), files) {
                once(format!("skill {name}: {err:#}"));
            }
        }
        for dir in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            if !skills.contains_key(&*dir.file_name().to_string_lossy()) && dir.path().join(marker()).is_file() {
                let _ = std::fs::remove_dir_all(dir.path());
            }
        }
    }
    written.save();
    Ok(())
}

fn write_skill(dir: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    let ours = |dir: &Path| dir.join(marker()).is_file();
    if dir.exists() && !ours(dir) && !dir.join(node_marker()).is_file() {
        once(format!("left {} alone: it isn't a synced skill", dir.display()));
        return Ok(());
    }
    let same = ours(dir) && files.iter().all(|(p, t)| std::fs::read_to_string(dir.join(p)).is_ok_and(|now| &now == t));
    if same {
        return Ok(());
    }
    for path in files.keys() {
        if path.starts_with('/') || path.contains('\\') || path.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
            bail!("{path} isn't a path inside the skill");
        }
    }
    let tmp = dir.with_extension("syncing");
    let _ = std::fs::remove_dir_all(&tmp);
    for (path, text) in files {
        let file = tmp.join(path);
        std::fs::create_dir_all(file.parent().context("no folder")?)?;
        std::fs::write(&file, text)?;
    }
    std::fs::write(tmp.join(marker()), "This skill is kept the same on every device. Change it with agents_skill_set; delete this file to take the folder over.\n")?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::rename(&tmp, dir)?;
    Ok(())
}

fn once(said: String) {
    static TOLD: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut told = TOLD.lock().unwrap_or_else(|e| e.into_inner());
    if !told.contains(&said) {
        eprintln!("agents: {said}");
        told.push(said);
    }
}

struct Written {
    path: PathBuf,
    hashes: HashMap<String, String>,
}

impl Written {
    fn load(host: &Host) -> Self {
        let path = host.dir.join("written.json");
        let hashes = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                once(format!("couldn't read {}, so every skill file counts as edited by hand: {err}", path.display()));
                HashMap::new()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(err) => {
                once(format!("couldn't read {}: {err}", path.display()));
                HashMap::new()
            }
        };
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

    fn save(&self) {
        let saved = serde_json::to_vec_pretty(&self.hashes).map_err(|err| err.to_string()).and_then(|bytes| std::fs::write(&self.path, bytes).map_err(|err| err.to_string()));
        if let Err(err) = saved {
            once(format!("couldn't save {}: {err}", self.path.display()));
        }
    }
}
