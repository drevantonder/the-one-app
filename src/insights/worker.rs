//! The telemetry Worker (`cloudflare/telemetry/worker.js`), as a device reaches it: a request signed by its own key.

use std::time::Duration;

use anyhow::{Result, ensure};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

use super::{Ask, Batch};
use crate::hedge::Twice;

/// How long a request may take: a device has more to do, and telemetry may be lost.
const TIMEOUT: Duration = Duration::from_secs(20);

fn post(address: &str, path: &str, key: &SigningKey, fields: Value) -> Result<Value> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
    let body = crate::keys::signed_request(key, crate::protocol::TELEMETRY, fields, crate::keys::next_at(now));
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIMEOUT)).build().into();
    let request = ureq::http::Request::post(format!("{}{path}", address.trim_end_matches('/'))).header("content-type", "application/json").body(serde_json::to_vec(&body)?)?;
    // A read may reach the Worker twice; rows written twice would be counted twice.
    let twice = if path.ends_with("/read") { Twice::Safe } else { Twice::Never };
    let (mut resp, _) = crate::hedge::run(&agent, "telemetry", request, twice)?;
    let status = resp.status().as_u16();
    let said: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    ensure!(status < 300, "telemetry said {status}: {}", said["error"].as_str().unwrap_or(""));
    Ok(said)
}

pub fn write(address: &str, key: &SigningKey, batch: &Batch) -> Result<()> {
    post(address, "/telemetry/write", key, batch.rows()).map(|_| ())
}

/// What's read for `ask`, of `services` alone.
pub fn asked(ask: &Ask, services: Vec<String>) -> Value {
    let queries: Vec<&str> = if ask.queries.is_empty() { vec!["requests", "spans", "logs", "metrics", "errors"] } else { ask.queries.clone() };
    json!({ "queries": queries, "services": services, "service": ask.service, "name": ask.name, "trace": ask.trace, "status": ask.status, "least": ask.least, "since": ask.since.unwrap_or(0), "limit": ask.limit.max(1).min(10_000) })
}

pub fn read(address: &str, key: &SigningKey, fields: Value) -> Result<Value> {
    post(address, "/telemetry/read", key, fields)
}
