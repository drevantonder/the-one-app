//! What a call hands back when it returns, as `src/app/runtime/shim.js` gives it, on a machine and on Workers alike.

use anyhow::{Context as _, Result};
use serde_json::Value;

/// What a call asks the node to write once it returns.
pub enum Append {
    /// A statement that writes the app's database of a scope, which keeps the blobs it names; or with `check`, a query
    /// whose first value must be true as the write runs, else the call fails with that message.
    Exec { scope: String, sql: String, params: Vec<Value>, blobs: Vec<String>, check: Option<String> },
    /// Adds (or removes) an app in a scope; added, with its service run on `node` (a device's name, or `cloudflare`).
    Install { app: String, scope: String, installed: bool, node: Option<String> },
    /// Switches one of an app's schedules on or off in a scope, with the caller's own rights there.
    Switch { app: String, schedule: String, scope: String, on: bool },
    /// Says a scope no longer needs a blob it kept: once no scope does, devices delete it.
    Forget { scope: String, blob: String },
    /// Deletes an app (`deleted`), restores it (`restored`) or erases it (`erased`), with the caller's own rights.
    Retire { app: String, how: String },
    /// Sets (or with an empty link, clears) where an app's code is kept, with the caller's own rights.
    Source { app: String, link: String },
    /// Runs an app's service on a device (or stops it there), for the app added in `scope` (by default where the caller
    /// sees it added): for the device's person, or to stop it, for who may add apps where it ran.
    Service { device: String, name: String, on: bool, scope: Option<String> },
    /// What the call spent on AI: `{ model, usd, input, output }`, for the insights of the app that called it.
    Cost { data: Value },
    /// Makes, changes or lets go of (`release`) a Vault item: `scope` for one it makes, `same` a plain field by
    /// which it changes an item of that type instead.
    Vault { id: String, scope: Option<String>, data: Value, same: Option<String>, release: bool },
}

pub fn appends(list: &Value) -> Result<Vec<Append>> {
    let mut appends = Vec::new();
    for a in list.as_array().into_iter().flatten() {
        let scope = a["scope"].as_str().unwrap_or("").to_string();
        if let Some(id) = a["vault"].as_str() {
            let scope = a["scope"].as_str().map(str::to_string);
            let data = if a["item"].is_object() { a["item"].clone() } else { serde_json::json!({}) };
            appends.push(Append::Vault { id: id.into(), scope, data, same: a["same"].as_str().map(str::to_string), release: a["release"] == true });
            continue;
        }
        if let Some(schedule) = a["switch"].as_str() {
            appends.push(Append::Switch { app: a["app"].as_str().unwrap_or("").into(), schedule: schedule.into(), scope, on: a["on"] != false });
            continue;
        }
        if let Some(name) = a["service"].as_str() {
            appends.push(Append::Service { device: a["device"].as_str().unwrap_or("").into(), name: name.into(), on: a["on"] == true, scope: a["scope"].as_str().map(str::to_string) });
            continue;
        }
        if let Some(name) = a["retire"].as_str() {
            appends.push(Append::Retire { app: name.into(), how: a["how"].as_str().unwrap_or("").into() });
            continue;
        }
        if let Some(name) = a["app_source"].as_str() {
            appends.push(Append::Source { app: name.into(), link: a["link"].as_str().unwrap_or("").into() });
            continue;
        }
        if a["cost"].is_object() {
            appends.push(Append::Cost { data: a["cost"].clone() });
            continue;
        }
        if let Some(blob) = a["forget"].as_str() {
            appends.push(Append::Forget { scope, blob: blob.into() });
            continue;
        }
        if let Some(name) = a["install"].as_str() {
            appends.push(Append::Install { app: name.into(), scope, installed: a["installed"] != false, node: a["node"].as_str().map(str::to_string) });
            continue;
        }
        let sql = a["exec"].as_str().context("an app writes its database with exec")?.to_string();
        let params = a["params"].as_array().cloned().unwrap_or_default();
        let blobs = a["blobs"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_string)).collect();
        appends.push(Append::Exec { scope, sql, params, blobs, check: a["check"].as_str().map(str::to_string) });
    }
    Ok(appends)
}
