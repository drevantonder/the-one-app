//! Backups' tools, each its own route (`api::Runs`): how they're going, backing up now, and the places
//! they go. Settings' Backups screen asks the same routes, and `/api/backups/place` besides, which takes the key of a
//! place its user types and so is not a tool.

use serde_json::json;

use ::custom_build::config::CLI;
use ::api::Feature;

const GUIDE: &str = "Each device backs up the data it holds, every hour that something changed, to each place: a folder on a device's disk, or S3-compatible storage. Everything is encrypted before it leaves, with keys only your devices and the recovery key open. To add S3 storage, call backups_add_place with its endpoint and give the user the link: they type the key on their own device. Snapshots are kept hourly for 2 days, daily for a month and monthly for a year. backups_status says how it's going, and what's wrong and its fix if anything is; Once a week each place's newest backup is restored into a throwaway copy and checked (who's who, databases, blobs); a failed check shows on Backups like a failed backup. backups_check runs that check now and says what came back.";

pub fn feature() -> Feature {
    Feature {
        name: "backups",
        title: "Backups",
        description: "Encrypted copies of everything, to a disk or S3-compatible storage".into(),
        guide: GUIDE.into(),
        tools: tools(),
    }
}

fn said(about: &str) -> serde_json::Value {
    json!({ "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": about } } })
}

fn tools() -> Vec<serde_json::Value> {
    let none = json!({ "type": "object", "properties": {} });
    vec![
        json!({
                "name": "status", "title": "How backups are going", "reads": true,
                "description": "How backups are going: each place backed up to (a folder on a device's disk, or S3-compatible storage), when it last backed up and by which device, what's failing and since when, and the recent snapshots.",
                "input": none,
                "output": { "type": "object", "required": ["places", "snapshots", "here", "disks"], "properties": {
                    "places": { "type": "array", "items": { "type": "object" }, "description": "each { id, title, kind, where, endpoint, device, problem, last, checked }" },
                    "snapshots": { "type": "array", "items": { "type": "object" }, "description": "the newest, each { time, devices, places, scopes }" },
                    "here": { "type": "string", "description": "this device's name" },
                    "disks": { "type": "array", "items": { "type": "object" }, "description": "the disks here a folder could be on" },
                    "said": { "type": "string", "description": "with no place yet, what to do first: the tool answers with this alone" },
                } },
            }),
        json!({
                "name": "now", "title": "Back up now",
                "description": "Back up now, without waiting for the hour: each device backs up what it holds to each place.",
                "input": none,
                "output": said("that it started"),
            }),
        json!({
                "name": "add_place", "title": "Add a place to back up to", "words": ["folder"],
                "description": "Add a place to back up to: S3-compatible storage (Cloudflare R2, Backblaze B2, MinIO…), or a folder on this device, like a USB disk's. For storage you give the endpoint; the user gets a link to a sheet on their own device where they type the bucket, access key ID and secret key, so you never see the key. A folder is tried before it's added. backups_status then shows its first backup, or what went wrong.",
                "input": { "type": "object", "properties": {
                    "endpoint": { "type": "string", "description": "like https://<account id>.r2.cloudflarestorage.com" },
                    "folder": { "type": "string", "description": "instead of endpoint: a folder on this device, by its absolute path, like /mnt/usb/backups" },
                    "title": { "type": "string", "description": "what the user calls it, like Cloudflare R2" },
                } },
                "output": said("how trying the folder went, or the link to give the user"),
            }),
        json!({
                "name": "remove_place", "title": "Stop backing up to a place", "confirm": true,
                "description": "Stop backing up to a place. What's already there stays.",
                "input": { "type": "object", "required": ["place"], "properties": { "place": { "type": "string", "description": "its title or id, as backups_status lists it" } } },
                "output": said("which place is no longer backed up to"),
            }),
        json!({
                "name": "check", "title": "Restore a backup to check it", "reads": true,
                "description": format!("Restore each place's newest backup into a throwaway copy now, as the weekly check does: who's who from its steps, every database to its newest step as the network has it, and every blob's hash. Says what came back, and what's wrong. Any backup on a disk is opened with the recovery key alone, with or without a network here, by the command {CLI} backups check --folder <path> --recovery-key <file>."),
                "input": { "type": "object", "properties": {} },
                "output": { "type": "array", "items": { "type": "object" }, "description": "each place's report" },
            }),
    ]
}

/// What trying a place did, a line for each step.
pub fn tried_said(r: &serde_json::Value) -> String {
    let mut out = String::new();
    for s in r["tried"].as_array().into_iter().flatten() {
        match s["error"].as_str() {
            None => out.push_str(&format!("✓ {}\n", s["step"].as_str().unwrap_or(""))),
            Some(e) => out.push_str(&format!("✗ {}: {e}\n", s["step"].as_str().unwrap_or(""))),
        }
    }
    out
}

/// What serves each tool, in this runtime.
#[cfg(feature = "node")]
pub fn runs() -> ::api::Runs {
    use crate::node::runtime::node_serves;
    ::api::Runs {
        tools: vec![
            ("status", node_serves!(serve::status)),
            ("now", node_serves!(serve::now)),
            ("add_place", node_serves!(serve::add_place)),
            ("remove_place", node_serves!(serve::remove_place)),
            ("check", node_serves!(serve::check)),
        ],
        ..::api::Runs::named("backups")
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::{Context, Result, bail, ensure};
    use serde_json::{Map, Value, json};

    use ::api::feature::text;
    use crate::node::Node;
    use ::api::Caller;

    pub async fn status(node: Node, me: Caller, _: Map<String, Value>) -> Result<Value> {
        node.only_admins_back_up(&me)?;
        node.backups()
    }

    pub async fn now(node: Node, me: Caller, _: Map<String, Value>) -> Result<Value> {
        node.only_admins_back_up(&me)?;
        node.back_up_now();
        Ok(json!({ "said": "Backing up now; backups_status says how it went." }))
    }

    /// A folder on this device, or S3-compatible storage, whose key the user types on their own device.
    pub async fn add_place(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        node.only_admins_back_up(&me)?;
        let given = |k: &str| input.get(k).and_then(Value::as_str);
        let title = input.get("title").cloned().unwrap_or(Value::Null);
        match (given("folder"), given("endpoint")) {
            (Some(folder), None) => {
                ensure!(std::path::Path::new(folder).is_absolute(), "give folder as an absolute path, like /mnt/usb/backups: {folder} isn't one");
                let tried = node.add_place(&me, &json!({ "path": folder, "title": title })).await?;
                let kept = if tried["id"].is_string() { "Added: the first backup starts now, then every hour that something changes." } else { "Not added." };
                Ok(json!({ "said": format!("{}{kept}", super::tried_said(&tried)) }))
            }
            (None, Some(endpoint)) => {
                let asked = node.request_place(&me, &json!({ "endpoint": endpoint, "title": title })).await?;
                Ok(json!({ "said": format!("Asked. Give the user this link: {}\nThey type the bucket and its key into a sheet on their own device, and you never see it. Once they have, backups_status shows its first backup.", asked["link"].as_str().unwrap_or("")) }))
            }
            _ => bail!("give endpoint, for S3-compatible storage, or folder, a folder on this device"),
        }
    }

    pub async fn remove_place(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        node.only_admins_back_up(&me)?;
        let place = text(&input, "place")?;
        let places = node.backups()?["places"].as_array().cloned().unwrap_or_default();
        let p = places.iter().find(|p| p["id"] == *place || p["title"] == *place).with_context(|| format!("no place {place}; backups_status lists them"))?;
        node.remove_place(&me, p["id"].as_str().unwrap_or(place)).await?;
        Ok(json!({ "said": format!("No longer backing up to {}. What's there stays.", p["title"].as_str().unwrap_or(place)) }))
    }

    pub async fn check(node: Node, me: Caller, _: Map<String, Value>) -> Result<Value> {
        node.only_admins_back_up(&me)?;
        node.check_backups().await
    }
}
