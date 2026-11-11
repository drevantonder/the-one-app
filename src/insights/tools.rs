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
                "description": "How the network and its apps are doing, from what you may see (an app's only if you may deploy it; a device's only if you're an admin), as a few lines to read: each service's requests, error rate and slowest calls (P95), and the top errors. Give service for that service's operations, its failing and slow traces and its readings (a gauge's newest, a counter's total), then trace_id for one trace's spans. For an admin, the overview also says what the network's use of Cloudflare cost in that time, estimated at list prices. Use it when something is failing, erroring or slow. raw gives the rows themselves instead: requests by the hour, spans, logs, metrics and errors as JSON, each up to limit, and traces, the newest traces with a span of service, name, status and min_ms over the whole range, each with all its spans. Never holds a call's arguments or answer; a real failure keeps what it said, cleaned of anything that names someone or opens something. Errors are what broke: a caller's mistake (refused, invalid input, no such thing) is status unset, listed only with status unset. What's newest takes up to a minute or two to show.",
                "input": { "type": "object", "properties": {
                    "service": { "type": "string", "description": "only this service: an app's name, or the network's own program" },
                    "trace_id": { "type": "string", "description": "one trace by its id, as its spans: a failure says its trace-id" },
                    "raw": { "type": "boolean", "description": "the rows themselves, as JSON" },
                    "effect_of": { "type": "string", "description": "what a build (its commit, or the start of it) changed against the build before it: newly failing and slower operations, or that there's nothing to compare yet" },
                    "parts": { "type": "array", "items": { "type": "string", "enum": ["requests", "spans", "logs", "metrics", "errors", "traces"] }, "description": "with raw, only these kinds of row (each costs a read of its own): every kind unless given" },
                    "check": { "type": "boolean", "description": "check telemetry's own path now, by sending a known signal and reading the last one back, before answering. It's checked every 15 minutes anyway" },
                    "name": { "type": "string", "description": "with raw, only spans of this name or logs of this body" },
                    "min_ms": { "type": "integer", "description": "with raw's traces, only those with a span that took at least this long, in ms" },
                    "since": { "type": "string", "description": "only the last this long: 30s, 10m, 2h or 1d (1d unless raw)" },
                    "status": { "type": "string", "enum": ["unset", "error"], "description": "only these: `error` is what broke (the default of the top errors listed), `unset` is what didn't, as what a caller had refused. With raw, also only spans of this status, and logs at error level if error or below it if unset" },
                    "limit": { "type": "integer", "description": "how many lines to list, or with raw, how many of the newest of each kind of row, up to 10000 (20 and 100 by default)" },
                } },
                "output": { "type": ["string", "object"], "description": "the lines to read, or with raw `{ requests, spans, logs, metrics, errors }`", "additionalProperties": true },
            }),
            serves!(serve::insights),
        )],
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value, json};

    use crate::insights::{Ask, cost, view};
    use crate::node::{Caller, Node};

    /// As much of a table as a reading of it needs: its lines are cut, never what they add up from.
    const ALL_ROWS: usize = 10_000;
    const DAY: u64 = 86_400_000;
    const ALL_PARTS: [&str; 6] = ["requests", "spans", "logs", "metrics", "errors", "traces"];

    /// Only what's read of it: each is a query of Basin's.
    fn queries(raw: bool, parts: Option<&Vec<Value>>, trace: bool, service: bool, effect: bool) -> Vec<&'static str> {
        match (raw, trace, service, effect) {
            (true, ..) => parts.into_iter().flatten().filter_map(|p| ALL_PARTS.iter().copied().find(|q| p == q)).collect(),
            (_, _, _, true) => vec!["builds", "build_errors"],
            (_, true, ..) => vec!["spans"],
            (_, _, true, _) => vec!["requests", "errors", "spans", "metrics"],
            _ => vec!["requests", "errors"],
        }
    }

    pub async fn insights(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
        let raw = input.get("raw").and_then(Value::as_bool).unwrap_or(false);
        let trace = text("trace_id");
        let effect = text("effect_of");
        let now = crate::node::now_ms();
        let since = match text("since").map(|s| crate::logs::since(&s)).transpose()? {
            Some(d) => Some(now.saturating_sub(d.as_millis() as u64)),
            None if raw || trace.is_some() => None,
            None if effect.is_some() => Some(now.saturating_sub(7 * DAY)),
            None => Some(now.saturating_sub(DAY)),
        };
        let service = text("service");
        let limit = input.get("limit").and_then(Value::as_u64).map(|n| n as usize);
        if input.get("check").and_then(Value::as_bool).unwrap_or(false) {
            node.check_telemetry().await;
        }
        let ask = Ask { queries: queries(raw, input.get("parts").and_then(Value::as_array), trace.is_some(), service.is_some(), effect.is_some()), service, name: text("name"), trace: trace.clone(), since, status: text("status"), least: input.get("min_ms").and_then(Value::as_u64), limit: if raw { limit.unwrap_or(100) } else { ALL_ROWS } };
        let overview = !raw && trace.is_none() && effect.is_none() && ask.service.is_none();
        // What the network's use of Cloudflare cost, beside the overview: an admin's only, as a service no one else may read.
        let usage = Ask { queries: vec!["metrics"], service: Some(cost::CLOUDFLARE.to_string()), name: None, trace: None, since, status: None, least: None, limit: ALL_ROWS };
        let (rows, usage) = tokio::join!(node.insights(&me, &ask), async { if overview { node.insights(&me, &usage).await.ok() } else { None } });
        let rows = rows?;
        if raw {
            return Ok(rows);
        }
        let limit = limit.unwrap_or(view::LISTED);
        let status = ask.status.as_deref().unwrap_or(view::ERROR);
        let text = match (&trace, &ask.service) {
            _ if effect.is_some() => view::effect(&rows, effect.as_deref().unwrap_or(""), now, limit),
            (Some(id), _) => view::trace(&rows, id, limit),
            (None, Some(service)) => view::service(&rows, service, status, since.unwrap_or(0), now, limit),
            (None, None) => {
                let since = since.unwrap_or(0);
                let metrics = usage.as_ref().and_then(|u| u["metrics"].as_array()).cloned().unwrap_or_default();
                let cost = cost::said(&metrics, &view::since_said(since, now));
                [Some(view::overview(&rows, status, since, now, limit)), cost].into_iter().flatten().collect::<Vec<_>>().join("\n\n")
            }
        };
        Ok(json!(view::with_problem(&rows, now, text)))
    }
}
