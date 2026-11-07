//! Backups' tools: how they're going, backing up now, and the places they go.

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use super::{Asked, Feature, Request, Tool, as_given};

const GUIDE: &str = "Each device backs up the data it holds, every hour that something changed, to each place: a folder on a device's disk, or S3-compatible storage. Everything is encrypted before it leaves, with keys only your devices and the recovery key open. To add S3 storage, call backups_add_place with its endpoint and give the person the link: they type the key on their own device. Snapshots are kept hourly for 2 days, daily for a month and monthly for a year. backups_status says how it's going, and what's wrong and its fix if anything is; Once a week each place's newest backup is restored into a throwaway copy and checked (who's who, databases, blobs); a failed check shows on Backups like a failed backup. backups_check runs that check now and says what came back.";

pub fn feature() -> Feature {
    Feature {
        name: "backups",
        title: "Backups",
        description: "Encrypted copies of everything, to a disk or S3-compatible storage".into(),
        guide: GUIDE.into(),
        tools: tools(),
        #[cfg(feature = "node")]
        steps: steps::run,
    }
}

fn tools() -> Vec<Tool> {
    let none = json!({ "type": "object", "properties": {} });
    vec![
        Tool::asks(json!({ "name": "status", "reads": true, "description": "How backups are going: each place backed up to (a folder on a device's disk, or S3-compatible storage), when it last backed up and by which device, what's failing and since when, and the recent snapshots.", "input": none }), |_, _| Ok(Request::get("/v1/backups")), as_given),
        Tool::asks(json!({ "name": "now", "description": "Back up now, without waiting for the hour: each device backs up what it holds to each place.", "input": none }), now, now_said),
        Tool::asks(json!({ "name": "add_place", "words": ["folder"], "description": "Add a place to back up to: S3-compatible storage (Cloudflare R2, Backblaze B2, MinIO…), or a folder on this device, like a USB disk's. For storage you give the endpoint; the person gets a link to a sheet on their own device where they type the bucket, access key ID and secret key, so you never see the key. A folder is tried before it's added. backups_status then shows its first backup, or what went wrong.", "input": { "type": "object", "properties": { "endpoint": { "type": "string", "description": "like https://<account id>.r2.cloudflarestorage.com" }, "folder": { "type": "string", "description": "instead of endpoint: a folder on this device, by its path" }, "title": { "type": "string", "description": "what the person calls it, like Cloudflare R2" } } } }), add_place, add_place_said),
        Tool::steps(json!({ "name": "remove_place", "confirm": true, "description": "Stop backing up to a place. What's already there stays.", "input": { "type": "object", "required": ["place"], "properties": { "place": { "type": "string", "description": "its title or id, as backups_status lists it" } } } })),
        Tool::steps(json!({ "name": "check", "reads": true, "description": "Restore each place's newest backup into a throwaway copy now, as the weekly check does: who's who from its steps, every database to its newest step as the network has it, and every blob's hash. Says what came back, and what's wrong. With folder, any backup on a disk instead, opened with the recovery key alone, with or without a network here.", "input": { "type": "object", "properties": { "folder": { "type": "string", "description": "a backup's folder on this device, by its path" }, "recovery_key": { "type": "string", "description": "with folder: the path of a file on this machine holding the recovery key (- reads it from stdin on the command line); never the key itself" } } } })),
    ]
}

fn now(_: &Map<String, Value>, asked: &Asked) -> Result<Request> {
    Ok(Request::post("/v1/backups/now", json!({ "agent": asked.agent })))
}

fn now_said(_: &Map<String, Value>, _: Value) -> Result<Value> {
    Ok(json!("Backing up now; backups_status says how it went."))
}

/// A folder on this device, or S3-compatible storage, whose key the person types on their own device.
fn add_place(input: &Map<String, Value>, asked: &Asked) -> Result<Request> {
    let given = |k: &str| input.get(k).and_then(Value::as_str);
    Ok(match (given("folder"), given("endpoint")) {
        (Some(folder), None) => Request::post("/v1/backups/place", json!({ "path": std::path::absolute(folder)?, "title": input.get("title"), "agent": asked.agent })),
        (None, Some(endpoint)) => Request::post("/v1/backups/request", json!({ "endpoint": endpoint, "title": input.get("title"), "agent": asked.agent })),
        _ => bail!("give endpoint, for S3-compatible storage, or folder, a folder on this device"),
    })
}

fn add_place_said(input: &Map<String, Value>, r: Value) -> Result<Value> {
    Ok(json!(match input.get("folder") {
        Some(_) => format!("{}{}", tried_said(&r), if r["id"].is_string() { "Added: the first backup starts now, then every hour that something changes." } else { "Not added." }),
        None => format!("Asked. Give the person this link: {}\nThey type the bucket and its key into a sheet on their own device, and you never see it. Once they have, backups_status shows its first backup.", r["link"].as_str().unwrap_or("")),
    }))
}

/// What trying a place did, a line for each step.
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

#[cfg(feature = "node")]
mod steps {
    use anyhow::{Context, Result, bail};
    use serde_json::{Map, Value, json};

    use super::Asked;
    use crate::api::Client;

    pub fn run(tool: &str, input: &Map<String, Value>, asked: &Asked, node: &Client) -> Result<Value> {
        let given = |k: &str| input.get(k).and_then(Value::as_str);
        match tool {
            "remove_place" => {
                let place = super::super::text(input, "place")?;
                let places = node.get("/v1/backups")?["places"].as_array().cloned().unwrap_or_default();
                let p = places.iter().find(|p| p["id"] == *place || p["title"] == *place).with_context(|| format!("no place {place}; backups_status lists them"))?;
                node.post("/v1/backups/place", json!({ "id": p["id"], "deleted": true, "agent": asked.agent }))?;
                Ok(json!(format!("No longer backing up to {}. What's there stays.", p["title"].as_str().unwrap_or(place))))
            }
            "check" => match given("folder") {
                Some(folder) => check_folder(folder, &super::super::devices::recovery_key_from(given("recovery_key").context("give recovery_key: the file the recovery key is in")?, asked.reader)?),
                None => node.post("/v1/backups/check", json!({})),
            },
            other => bail!("no tool backups_{other}"),
        }
    }

    /// Checks a backup on a disk with the recovery key alone, here, without a device: every network it holds.
    fn check_folder(folder: &str, recovery_key: &str) -> Result<Value> {
        let secret = crate::keys::unhex(recovery_key).context("a recovery key is 64 hex characters")?;
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
}
