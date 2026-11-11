//! What serves Device logs' tool (device-logs/src/tools.rs).

use anyhow::Result;
use api::Caller;
use api::call::CallRequest;
use api::feature::{say_time, text};
use api::runtime::prelude::*;
use api::serves;
use serde_json::{Map, Value, json};

/// What serves each of its tools (device-logs/src/tools.rs).
pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![("logs", serves!(logs))]
}

/// A device's log, or one of its services', as `logs` asks for it (`device`, `service`, `since`, `grep`, `lines`): the
/// app runtime there says no unless the caller may remove that device (app-runtime/src/calls/running.rs).
async fn logs(rt: Rt, me: Caller, mut input: Map<String, Value>) -> Result<Value> {
    let device = text(&input, "device")?.to_string();
    input.remove("device");
    device_logs::Ask::from_json(&Value::Object(input.clone()))?;
    let read = CallRequest { app: key::APP.into(), tool: "logs".into(), device: Some(device.clone()), scopes: Some(Vec::new()), input: Value::Object(input), agent: me.agent.clone(), ..Default::default() };
    let r = rt.call(read, me).await?;
    let (shown, matched) = (r["shown"].as_u64().unwrap_or(0), r["matched"].as_u64().unwrap_or(0));
    let note = if shown == 0 {
        let from = r["oldest"].as_u64().map(|ms| format!("; its log goes back to {}", say_time(ms))).unwrap_or_default();
        Some(format!("{device}'s {} log has no lines like that{from}.", r["log"].as_str().unwrap_or("node")))
    } else if matched > shown {
        Some(format!("{shown} of the {matched} lines that match, the newest: narrow it with since or grep."))
    } else {
        None
    };
    let lines: Vec<&str> = r["lines"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    Ok(json!({ "said": format!("{}{}", lines.join("\n"), note.map(|n| format!("\n{n}")).unwrap_or_default()) }))
}
