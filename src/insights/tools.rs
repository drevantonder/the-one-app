//! Insights, as the core's tool `insights`, its own route (`core_tools::Runs::Route`).

use serde_json::json;

use crate::core_tools::{Feature, Tool, serves};

pub fn feature() -> Feature {
    Feature {
        name: "insights",
        title: "Insights",
        description: "How your network and apps are doing: requests, errors, slow calls and traces".into(),
        guide: String::new(),
        tools: vec![Tool::route(
            json!({
                "name": "insights", "title": "Read how things went", "reads": true,
                "description": "How the network and its apps are doing, from what you may see (an app's only if you may deploy it; a device's only if you're an admin), as a few lines to read: each service's requests, error rate and slowest calls (P95), and the top errors. Give service for that service's operations and its failing and slow traces, then trace_id for one trace's spans. Use it when something is failing, erroring or slow. raw gives the rows themselves instead: signals, hourly rollups and error fingerprints as JSON, each up to limit. Never holds a call's arguments or answer; a real failure keeps what it said, cleaned of anything that names someone or opens something, for a week. Errors are what broke: a caller's mistake (refused, invalid input, no such thing) is status unset, listed only with status unset.",
                "input": { "type": "object", "properties": {
                    "service": { "type": "string", "description": "only this service: an app's name, or the network's own program" },
                    "trace_id": { "type": "string", "description": "one trace by its id, as its spans" },
                    "raw": { "type": "boolean", "description": "the rows themselves, as JSON" },
                    "name": { "type": "string", "description": "with raw, only signals of this name, like search" },
                    "since": { "type": "string", "description": "only the last this long: 30s, 10m, 2h or 1d (1d unless raw)" },
                    "status": { "type": "string", "enum": ["unset", "error"], "description": "only these: `error` is what broke (the default of the top errors listed), `unset` is what didn't, as what a caller had refused. With raw, also only signals of this status, and only rollups with errors if error" },
                    "limit": { "type": "integer", "description": "how many lines to list, or with raw, how many of the newest of each kind of row (20 and 100 by default)" },
                } },
                "output": { "type": ["string", "object"], "description": "the lines to read, or with raw `{ signals, rollups, fingerprints, more_signals, more_rollups, more_fingerprints }`", "additionalProperties": true },
            }),
            serves!(serve::insights),
        )],
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value, json};

    use crate::insights::{Ask, view};
    use crate::node::{Caller, Node};

    /// As much of a table as a reading of it needs: its lines are cut, never what they add up from.
    const ALL_ROWS: usize = usize::MAX;
    const DAY: u64 = 86_400_000;

    pub async fn insights(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
        let raw = input.get("raw").and_then(Value::as_bool).unwrap_or(false);
        let trace = text("trace_id");
        let now = crate::node::now_ms();
        let since = match text("since").map(|s| crate::logs::since(&s)).transpose()? {
            Some(d) => Some(now.saturating_sub(d.as_millis() as u64)),
            None if raw || trace.is_some() => None,
            None => Some(now.saturating_sub(DAY)),
        };
        let service = text("service").map(|s| if s == crate::config::CLI { String::new() } else { s });
        let limit = input.get("limit").and_then(Value::as_u64).map(|n| n as usize);
        let ask = Ask { app: service, name: text("name"), trace: trace.clone(), since, status: text("status"), limit: if raw { limit.unwrap_or(100) } else { ALL_ROWS } };
        let rows = node.insights(&me, &ask).await?;
        if raw {
            return Ok(rows);
        }
        let limit = limit.unwrap_or(view::LISTED);
        let status = ask.status.as_deref().unwrap_or(view::ERROR);
        let text = match (&trace, &ask.app) {
            (Some(id), _) => view::trace(&rows, id, limit),
            (None, Some(service)) => view::service(&rows, input.get("service").and_then(Value::as_str).unwrap_or(service), status, since.unwrap_or(0), now, limit),
            (None, None) => view::overview(&rows, status, since.unwrap_or(0), now, limit),
        };
        Ok(json!(text))
    }
}
