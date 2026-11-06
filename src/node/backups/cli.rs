//! Backups' commands: `backups`, and `backups <what>`.

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::cli::{Command, Given};
use crate::config::CLI;

pub fn commands() -> Command {
    Command {
        help: vec![
            ("backups".into(), "how backups are going: each place, its last backup, what's failing".into()),
            ("backups now".into(), "back up now, without waiting for the hour".into()),
            ("backups place add <folder> [--title t]".into(), "back up to a folder on this device, like a USB disk's: tried before it's added".into()),
            ("backups place add --endpoint <url> [--title t]".into(), "back up to S3-compatible storage (R2, B2, MinIO): piped a JSON object of bucket, key_id and secret (and region) it tries and adds it; otherwise it asks the person for them, with a link to a sheet on their device".into()),
            ("backups place remove <place>".into(), "stop backing up to a place (what's there stays)".into()),
            ("backups check [--folder <path>]".into(), "restore each place's newest backup into a throwaway copy, check every entry and blob, and say what came back; with --folder, any backup on a disk, opened with the recovery key on stdin, with or without a network here".into()),
        ],
        run,
    }
}

fn run(g: &Given) -> Result<bool> {
    match g.words().as_slice() {
        ["backups"] => {
            g.takes_only(&[])?;
            print!("{}", backups_said(&g.node.get("/v1/backups")?));
        }
        ["backups", "now"] => {
            g.takes_only(&[])?;
            g.node.post("/v1/backups/now", json!({}))?;
            println!("Backing up now. {CLI} backups says how it went.");
        }
        ["backups", "place", "add"] | ["backups", "place", "add", _] => {
            g.takes_only(&["title", "endpoint"])?;
            let title = g.flags.get("title").cloned();
            let r = match (g.words.get(3), g.flags.get("endpoint").and_then(Value::as_str)) {
                (Some(path), None) => g.node.post("/v1/backups/place", json!({ "path": std::path::absolute(path)?, "title": title }))?,
                (None, Some(endpoint)) if !std::io::IsTerminal::is_terminal(&std::io::stdin()) => {
                    let mut body: Value = serde_json::from_reader(std::io::stdin()).context("pipe a JSON object of bucket, key_id and secret")?;
                    body["endpoint"] = json!(endpoint);
                    body["title"] = title.unwrap_or(Value::Null);
                    g.node.post("/v1/backups/place", body)?
                }
                (None, Some(endpoint)) => {
                    let r = g.node.post("/v1/backups/request", json!({ "endpoint": endpoint, "title": title, "agent": g.agent }))?;
                    println!("Open this on your device to give the bucket and its key: {}\nIt's also waiting in Settings, under Requests.", r["link"].as_str().unwrap_or(""));
                    return Ok(true);
                }
                _ => anyhow::bail!("give a folder, or --endpoint for S3-compatible storage"),
            };
            print!("{}", tried_said(&r));
            println!("{}", if r["id"].is_string() { "Added: the first backup starts now, then every hour that something changes." } else { "Not added." });
        }
        ["backups", "place", "remove", place] => {
            g.takes_only(&["confirm"])?;
            crate::cli::agreed(g.has("confirm"), &format!("{CLI} backups place remove"))?;
            let asked = crate::node::own::Asked { node: g.node, agent: None, client: "cli", confirmed: true };
            println!("No longer backing up to {}. What's there stays.", super::app::remove_place(&asked, place)?);
        }
        ["backups", "check"] => {
            g.takes_only(&["folder"])?;
            let r = match g.flags.get("folder").and_then(Value::as_str) {
                Some(folder) => check_folder(folder)?,
                None => g.node.post("/v1/backups/check", json!({}))?,
            };
            for place in r.as_array().into_iter().flatten() {
                print!("{}", checked_said(place));
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Backups' status, for people.
fn backups_said(r: &Value) -> String {
    let mut out = String::new();
    let places = r["places"].as_array().cloned().unwrap_or_default();
    if places.is_empty() {
        return format!("Nothing is backed up yet: add a place with {CLI} backups place add.\n");
    }
    for p in &places {
        let when = |t: &Value| t.as_u64().map(|t| crate::node::own::say_time(t)).unwrap_or_default();
        let last = match &p["last"] {
            Value::Null => "not backed up yet".to_string(),
            l => format!("backed up {} by {}", when(&l["time"]), l["device"].as_str().unwrap_or("")),
        };
        out.push_str(&format!("{} ({}): {last}\n", p["title"].as_str().unwrap_or(""), p["where"].as_str().unwrap_or("")));
        if let Some(problem) = p["problem"].as_object() {
            out.push_str(&format!("  {} {} (since {})\n", problem["text"].as_str().unwrap_or(""), problem["fix"].as_str().unwrap_or(""), when(&problem["since"])));
        }
    }
    out
}

/// Each step of trying a place, and whether it was added.
pub fn tried_said(r: &Value) -> String {
    let mut out = String::new();
    for s in r["tried"].as_array().into_iter().flatten() {
        match s["error"].as_str() {
            None => out.push_str(&format!("✓ {}\n", s["step"].as_str().unwrap_or(""))),
            Some(e) => out.push_str(&format!("✗ {}: {e}\n", s["step"].as_str().unwrap_or(""))),
        }
    }
    out
}

/// What a check restored, by name.
fn checked_said(r: &Value) -> String {
    let problems: Vec<&str> = r["problems"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if let Some(e) = r["error"].as_str().filter(|_| problems.is_empty()) {
        return format!("✗ {}: {e}\n", r["place"].as_str().unwrap_or(""));
    }
    if !problems.is_empty() {
        return format!("✗ {}: its newest backup didn't restore as it should:\n{}", r["place"].as_str().unwrap_or(""), problems.iter().map(|p| format!("  ✗ {p}\n")).collect::<String>());
    }
    let list = |k: &str| r[k].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
    let place = r["place"].as_str().unwrap_or("A disk");
    if r["scopes"].as_array().is_none_or(Vec::is_empty) {
        return match list("unchecked").as_str() {
            "" => format!("{place}: nothing backed up there yet\n"),
            unchecked => format!("{place}: not checked here, without their keys: {unchecked}\n"),
        };
    }
    let scopes: Vec<String> = r["scopes"].as_array().into_iter().flatten().map(|s| format!("{} ({} databases, {} blobs, from {})", s["name"].as_str().or(s["scope"].as_str()).unwrap_or(""), s["databases"], s["blobs"], crate::node::own::say_time(s["time"].as_u64().unwrap_or(0)))).collect();
    let mut out = format!(
        "{}: {} came back, checked, from {:.1} MB there\n  ✓ who's who from its steps, every blob as its hash says: {}\n",
        place,
        r["network"].as_str().unwrap_or(""),
        r["bytes"].as_f64().unwrap_or(0.0) / 1e6,
        scopes.join("; "),
    );
    if r["databases"] == true {
        out.push_str("  ✓ every database restores to its newest step, as the network has it\n");
    }
    out.push_str(&format!("  ✓ people: {}\n  ✓ apps: {}\n", list("people"), list("apps")));
    if !list("unchecked").is_empty() {
        out.push_str(&format!("  – not checked here, without their keys: {}\n", list("unchecked")));
    }
    out
}

/// Checks a backup on a disk with the recovery key alone, here, without a device: every network it holds.
fn check_folder(folder: &str) -> Result<Value> {
    let mut key = String::new();
    std::io::stdin().read_line(&mut key)?;
    let secret = crate::keys::unhex(key.trim()).context("give the recovery key on stdin: 64 hex characters")?;
    let place = crate::bucket::Place::Folder(folder.into());
    let mut out = Vec::new();
    for network in crate::backups::networks(&place)? {
        let keys = crate::backups::open_keys(&place, &network, &secret)?;
        let mut r = crate::backups::check(&crate::backups::Check { place: &place, network: &network, keys: &keys, root: None })?.0;
        r["place"] = json!(folder);
        out.push(r);
    }
    anyhow::ensure!(!out.is_empty(), "no backup in {folder}");
    Ok(json!(out))
}
