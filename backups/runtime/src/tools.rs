//! What serves Backups' tools (their specs are backups/src/tools.rs).

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};

use api::Caller;
use api::feature::text;
use api::runtime::prelude::*;
use api::serves;

use crate::run;

pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![("status", serves!(status)), ("now", serves!(now)), ("add_place", serves!(add_place)), ("remove_place", serves!(remove_place)), ("check", serves!(check))]
}

async fn status(rt: Rt, me: Caller, _: Map<String, Value>) -> Result<Value> {
    run::only_admins(&rt, &me)?;
    run::status(&rt).await
}

async fn now(rt: Rt, me: Caller, _: Map<String, Value>) -> Result<Value> {
    run::only_admins(&rt, &me)?;
    run::back_up_now(&rt);
    Ok(json!({ "said": "Backing up now; backups_status says how it went." }))
}

/// A folder on this device, or S3-compatible storage, whose key the user types on their own device.
async fn add_place(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    run::only_admins(&rt, &me)?;
    let given = |k: &str| input.get(k).and_then(Value::as_str);
    let title = input.get("title").cloned().unwrap_or(Value::Null);
    match (given("folder"), given("endpoint")) {
        (Some(folder), None) => {
            ensure!(std::path::Path::new(folder).is_absolute(), "give folder as an absolute path, like /mnt/usb/backups: {folder} isn't one");
            let tried = run::add_place(&rt, &me, &json!({ "path": folder, "title": title })).await?;
            let kept = if tried["id"].is_string() { "Added: the first backup starts now, then every hour that something changes." } else { "Not added." };
            Ok(json!({ "said": format!("{}{kept}", backups::tools::tried_said(&tried)) }))
        }
        (None, Some(endpoint)) => {
            let asked = run::request_place(&rt, &me, &json!({ "endpoint": endpoint, "title": title })).await?;
            Ok(json!({ "said": format!("Asked. Give the user this link: {}\nThey type the bucket and its key into a sheet on their own device, and you never see it. Once they have, backups_status shows its first backup.", asked["link"].as_str().unwrap_or("")) }))
        }
        _ => bail!("give endpoint, for S3-compatible storage, or folder, a folder on this device"),
    }
}

async fn remove_place(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    run::only_admins(&rt, &me)?;
    let place = text(&input, "place")?;
    let places = run::status(&rt).await?["places"].as_array().cloned().unwrap_or_default();
    let p = places.iter().find(|p| p["id"] == *place || p["title"] == *place).with_context(|| format!("no place {place}; backups_status lists them"))?;
    run::remove_place(&rt, &me, p["id"].as_str().unwrap_or(place)).await?;
    Ok(json!({ "said": format!("No longer backing up to {}. What's there stays.", p["title"].as_str().unwrap_or(place)) }))
}

async fn check(rt: Rt, me: Caller, _: Map<String, Value>) -> Result<Value> {
    run::only_admins(&rt, &me)?;
    run::check_backups(&rt).await
}
