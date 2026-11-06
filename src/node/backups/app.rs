//! Backups as an app: how they're going, backing up now, and the places they go.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::node::own::{Asked, NodeApp, text};
use crate::skills::{confirm_input, hints};

const GUIDE: &str = "Each device backs up the data it holds, every hour that something changed, to each place: a folder on a device's disk, or S3-compatible storage. Everything is encrypted before it leaves, with keys only your devices and the recovery key open. To add S3 storage, call backups_add_place with its endpoint and give the person the link: they type the key on their own device. Snapshots are kept hourly for 2 days, daily for a month and monthly for a year. backups_status says how it's going, and what's wrong and its fix if anything is; Once a week each place's newest backup is restored into a throwaway copy and checked (entries, databases, blobs); a failed check shows on Backups like a failed backup. backups_check runs that check now and says what came back.";

pub fn app() -> NodeApp {
    NodeApp {
        name: "backups",
        title: "Backups",
        description: "Encrypted copies of everything, to a disk or S3-compatible storage".into(),
        guide: GUIDE.into(),
        tools: tools(),
        // This machine's to run: never someone signed in from elsewhere.
        elsewhere: false,
        run,
    }
}

fn tools() -> Vec<Value> {
    vec![
        json!({ "name": "backups_status", "description": "How backups are going: each place backed up to (a folder on a device's disk, or S3-compatible storage), when it last backed up and by which device, what's failing and since when, and the recent snapshots.", "inputSchema": { "type": "object", "properties": {} }, "annotations": hints(true, false) }),
        json!({ "name": "backups_now", "description": "Back up now, without waiting for the hour: each device backs up what it holds to each place.", "inputSchema": { "type": "object", "properties": {} }, "annotations": hints(false, false) }),
        json!({ "name": "backups_add_place", "description": "Add S3-compatible storage (Cloudflare R2, Backblaze B2, MinIO…) to back up to. You give the endpoint; the person gets a link to a sheet on their own device where they type the bucket, access key ID and secret key, so you never see the key. backups_status then shows its first backup, or what went wrong.", "inputSchema": { "type": "object", "required": ["endpoint"], "properties": { "endpoint": { "type": "string", "description": "like https://<account id>.r2.cloudflarestorage.com" }, "title": { "type": "string", "description": "what the person calls it, like Cloudflare R2" } } }, "annotations": hints(false, false) }),
        json!({ "name": "backups_remove_place", "description": "Stop backing up to a place. What's already there stays.", "inputSchema": { "type": "object", "required": ["place"], "properties": { "place": { "type": "string", "description": "its title or id, as backups_status lists it" }, "confirm": confirm_input() } }, "annotations": hints(false, true) }),
        json!({ "name": "backups_check", "description": "Restore each place's newest backup into a throwaway copy now, as the weekly check does: who's who from its steps, every database to its newest step as the network has it, and every blob's hash. Says what came back, and what's wrong.", "inputSchema": { "type": "object", "properties": {} }, "annotations": hints(true, false) }),
    ]
}

fn run(tool: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value> {
    let (node, agent) = (asked.node, asked.agent);
    match tool {
        "status" => node.get("/v1/backups"),
        "now" => node.post("/v1/backups/now", json!({ "agent": agent })).map(|_| json!("Backing up now; backups_status says how it went.")),
        "add_place" => {
            let r = node.post("/v1/backups/request", json!({ "endpoint": input.get("endpoint"), "title": input.get("title"), "agent": agent }))?;
            Ok(json!(format!("Asked. Give the person this link: {}\nThey type the bucket and its key into a sheet on their own device, and you never see it. Once they have, backups_status shows its first backup.", r["link"].as_str().unwrap_or(""))))
        }
        "remove_place" => {
            remove_place(asked, text(input, "place")?)?;
            Ok(json!("Removed. What's there stays."))
        }
        "check" => node.post("/v1/backups/check", json!({})),
        other => bail!("no tool backups_{other}"),
    }
}

pub fn remove_place(asked: &Asked, place: &str) -> Result<String> {
    let places = asked.node.get("/v1/backups")?["places"].as_array().cloned().unwrap_or_default();
    let p = places.iter().find(|p| p["id"] == *place || p["title"] == *place).with_context(|| format!("no place {place}; backups_status lists them"))?;
    asked.node.post("/v1/backups/place", json!({ "id": p["id"], "deleted": true, "agent": asked.agent }))?;
    Ok(p["title"].as_str().unwrap_or("").to_string())
}
