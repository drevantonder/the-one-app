//! Where the network keeps its data: `storage`, and `storage move`.

use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::{Command, Given};
use crate::config::CLI;
use crate::backups::tools::tried_said;

pub fn commands() -> Command {
    Command {
        help: vec![
            ("storage".into(), "where the network's data is kept".into()),
            ("storage move --endpoint <url> --bucket <name> [--region <r>]".into(), "move it to S3-compatible storage (R2, S3, GCS, MinIO), whose key is the Workers' BUCKET: every device reaches it through the URL signer, and moves there".into()),
        ],
        run,
    }
}

fn run(g: &Given) -> Result<bool> {
    match g.words().as_slice() {
        ["storage"] => {
            let s = g.node.get("/v1/storage")?;
            match s["kind"].as_str() {
                Some("folder") => println!("The network's data is kept in a folder on {}: {}. Move it to S3-compatible storage before other devices join: {CLI} storage move --endpoint <url> --bucket <name>.", s["device"].as_str().unwrap_or(""), s["where"].as_str().unwrap_or("")),
                Some(_) => println!("The network's data is kept in {} at {}.", s["where"].as_str().unwrap_or(""), s["endpoint"].as_str().unwrap_or("")),
                None => println!("This device can't reach the network's data: an admin moves it to S3-compatible storage with {CLI} storage move."),
            }
        }
        ["storage", "move"] => {
            let get = |k: &str| g.flag(k);
            let endpoint = get("endpoint").context("give --endpoint, like https://<account>.r2.cloudflarestorage.com")?;
            let bucket = get("bucket").context("give --bucket, the bucket's name: its key is the Workers' (BUCKET, which only the URL signer holds)")?;
            let r = g.node.post("/v1/storage", json!({ "endpoint": endpoint, "bucket": bucket, "region": get("region") }))?;
            print!("{}", tried_said(&r));
            if r["kept"] == true {
                println!("Every device moves the network's data there now.");
            }
            if r["moving"] == true {
                println!("This device is putting what it holds there, and keeps the network's data there from now on. Do the same on each of your other devices.");
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
