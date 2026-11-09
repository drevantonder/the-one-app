//! Logs, as the core's tool `logs`, its own route (`core_tools::Runs::Route`): a device's log, or one of its services',
//! read over the network.

use serde_json::json;

use crate::core_tools::{Feature, Tool, serves};

pub fn feature() -> Feature {
    Feature {
        name: "logs",
        title: "Logs",
        description: "A device's log, or one of its services', over the network".into(),
        guide: String::new(),
        tools: vec![Tool::route(
            json!({
                "name": "logs", "title": "Read a device's log", "reads": true,
                "description": "Read a device's log over the network, instead of ssh: its node's, or one of its services', or for a phone or computer app, what its app and page say went wrong (read while the app is open and connected). The newest lines (200 by default, at most 1000 or 48 KiB), oldest first. Only the device's user and the network's admins may.",
                "input": { "type": "object", "required": ["device"], "properties": {
                    "device": { "type": "string", "description": "the device's name (devices_requests and groups_list name them; the status command lists them)" },
                    "service": { "type": "string", "description": "a service's name, like whatsapp; omit for the node's own log, and for an app's" },
                    "since": { "type": "string", "description": "only the last this long: 30s, 10m, 2h or 1d" },
                    "grep": { "type": "string", "description": "only lines with this text in them, whatever its case" },
                    "lines": { "type": "integer", "description": "how many of the newest to give" },
                } },
                "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": "the lines, oldest first, then a note when there are none or more than shown" } } },
            }),
            serves!(serve::logs),
        )],
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value, json};

    use crate::core_tools::{say_time, text};
    use crate::database::scopes;
    use crate::node::{CallRequest, Caller, Node};

    /// A device's log, or one of its services', as `logs` asks for it (`device`, `service`, `since`, `grep`, `lines`): the
    /// node there says no unless the caller may remove that device (src/node/calls/running.rs).
    pub async fn logs(node: Node, me: Caller, mut input: Map<String, Value>) -> Result<Value> {
        let device = text(&input, "device")?.to_string();
        input.remove("device");
        crate::logs::Ask::from_json(&Value::Object(input.clone()))?;
        let read = CallRequest { app: scopes::APP.into(), tool: "logs".into(), device: Some(device.clone()), scopes: Some(Vec::new()), input: Value::Object(input), agent: me.agent.clone(), ..Default::default() };
        let r = node.call_as(read, me).await?;
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
}
