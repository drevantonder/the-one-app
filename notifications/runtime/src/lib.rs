//! Notifications (`notifications`), which every app runtime sends itself for any caller, through the network's
//! notifications Worker: never a service a device runs.

mod tools;

use std::time::Duration;

use anyhow::{Result, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde_json::{Value, json};

use api::runtime::Runs;
use api::runtime::prelude::*;
use custom_build::config::YOURS;
use custom_build::now_ms;
use key::{Key, hex};
use notifications::Note;

pub fn runs() -> Runs {
    Runs { tools: tools::served(), ..Runs::named("notifications") }
}

/// Sends `note` to each of `user`'s devices, sealed to each: → how many it went to, and how many of their devices get
/// notifications.
pub async fn send(rt: &Rt, user: &str, note: &Note) -> Result<Value> {
    let key = rt.signing_key();
    let now = now_ms();
    let notes: Vec<Value> = devices_of(rt, user)?
        .into_iter()
        .map(|(to, seal, _)| Ok(json!({ "to": hex(&to), "note": B64U.encode(&notifications::seal_note(&key, &seal, note, now)?) })))
        .collect::<Result<_>>()?;
    let said = ask(rt, "/notifications/send", json!({ "notes": notes })).await?;
    let to = said["to"].as_object().cloned().unwrap_or_default();
    let sent = to.values().filter(|v| v["sent"] == true).count() as u64;
    let devices = to.values().filter(|v| v["none"].is_null() && v["gone"].is_null()).count() as u64;
    let mut out = json!({ "sent": sent, "devices": devices });
    if devices == 0 {
        out["why"] = json!("none of your devices get notifications yet: allow them in the app or the browser");
    } else if let Some(why) = to.values().find_map(|v| v["failed"].as_str()) {
        out["why"] = json!(why);
    }
    Ok(out)
}

/// The caller's devices that get notifications, as the notifications Worker has them registered.
async fn devices(rt: &Rt, user: &str) -> Result<Value> {
    let mine = devices_of(rt, user)?;
    let said = ask(rt, "/notifications/devices", json!({ "user": user, "keys": mine.iter().map(|(k, _, _)| hex(k)).collect::<Vec<_>>() })).await?;
    let name = |key: &str| mine.iter().find(|(k, _, _)| hex(k) == key).map(|(_, _, n)| n.clone()).unwrap_or_default();
    let out: Vec<Value> = said["registered"].as_array().into_iter().flatten().map(|d| json!({ "name": name(d["key"].as_str().unwrap_or("")), "via": d["via"], "since": d["at"] })).collect();
    Ok(json!(out))
}

/// Which push services the network's notifications Worker sends through.
async fn status(rt: &Rt) -> Result<Value> {
    let address = rt.address()?;
    let said = tokio::task::spawn_blocking(move || -> Result<Value> { Ok(platform::hedge::run(&agent(), "notifications", ureq::http::Request::get(format!("{address}/notifications/key")).body(Vec::new())?, platform::hedge::Twice::Safe)?.0.body_mut().read_json()?) }).await??;
    Ok(json!({ "via": said["via"] }))
}

/// A user's own devices that might get notifications, as the network's signed steps list them: (key, seal key, name).
fn devices_of(rt: &Rt, user: &str) -> Result<Vec<(Key, Key, String)>> {
    rt.read(|r| Ok(r.db().clients()?.into_iter().filter(|m| m.user == user && !m.removed && m.passkey.is_none()).map(|m| (m.key, m.seal, m.name)).collect()))
}

/// Asks the notifications Worker something as this runtime, signed with its key (`one/notifications`). → its answer.
async fn ask(rt: &Rt, path: &str, mut fields: Value) -> Result<Value> {
    let (address, key, path) = (rt.address()?, rt.signing_key(), path.to_string());
    tokio::task::spawn_blocking(move || {
        fields["route"] = json!(format!("POST {path}"));
        let signed = key::signed_request(&key, custom_build::protocol::NOTIFICATIONS, fields, key::next_at(now_ms()));
        let request = ureq::http::Request::post(format!("{address}{path}")).header("content-type", "application/json").body(serde_json::to_vec(&signed)?)?;
        let (mut resp, _) = platform::hedge::run(&agent(), "notifications", request, platform::hedge::Twice::Never)?;
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        ensure!(resp.status().is_success(), "{}", out["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("{YOURS}'s notifications didn't answer")));
        Ok(out)
    })
    .await?
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into()
}
