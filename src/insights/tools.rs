//! Insights, as the core's tool `insights`, its own route (`core_tools::Runs::Route`).

use serde_json::json;

use crate::core_tools::{Feature, Tool, serves};

pub fn feature() -> Feature {
    Feature {
        name: "insights",
        title: "Insights",
        description: "How your network and apps are doing: recent signals, hourly counts and errors".into(),
        guide: String::new(),
        tools: vec![Tool::route(
            json!({
                "name": "insights", "title": "Read how things went", "reads": true,
                "description": "Read what the network recorded of how things went, from what you may see: an app's only if you may deploy it; a device's only if you're an admin. Failures and slow calls as signals, every call in hourly rollups, and each error once as a fingerprint. Never holds a call's arguments or answer, or an app's error text. `fault` tells a caller's mistake (refused, invalid input, no such thing) from something that broke (`internal`): filter by it. Times are milliseconds since 1970.",
                "input": { "type": "object", "properties": {
                    "app": { "type": "string", "description": "only this app's" },
                    "name": { "type": "string", "description": "only signals of this name, like gmail.search" },
                    "since": { "type": "string", "description": "only the last this long: 30s, 10m, 2h or 1d" },
                    "status": { "type": "string", "enum": ["ok", "error"], "description": "only these signals; only rollups with errors if error" },
                    "fault": { "type": "string", "enum": ["caller", "internal"] },
                    "limit": { "type": "integer", "description": "how many of the newest signals to give, at most 500" },
                } },
                "output": { "type": "object", "required": ["signals", "rollups", "fingerprints"], "properties": {
                    "signals": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                    "rollups": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                    "fingerprints": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                } },
            }),
            serves!(serve::insights),
        )],
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value};

    use crate::insights::Ask;
    use crate::node::{Caller, Node};

    pub async fn insights(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
        let since = text("since").map(|s| crate::logs::since(&s)).transpose()?.map(|d| crate::node::now_ms().saturating_sub(d.as_millis() as u64));
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
        node.insights(&me, &Ask { app: text("app"), name: text("name"), since, status: text("status"), fault: text("fault"), limit }).await
    }
}
