//! Insights, as the core's tool `insights`, its own route (`api::Runs`).

use serde_json::json;

use ::api::Feature;

pub fn feature() -> Feature {
    Feature {
        name: "insights",
        title: "Insights",
        description: "How your network and apps are doing: requests, errors, slow calls and traces".into(),
        guide: String::new(),
        tools: vec![json!({
                "name": "insights", "title": "Read how things went", "reads": true,
                "description": "How the network and its apps are doing, from what you may see (an app's only if you may deploy it; a device's only if you're an admin), as a few lines to read: each service's requests, error rate and slowest calls (P95), and the top errors. Give service for that service's operations, its failing and slow traces and its readings (a gauge's newest, a counter's total), then trace_id for one trace's spans. For an admin, the overview also says what the network's use of Cloudflare cost in that time, estimated at list prices. The overview says what AI Gateway's calls spent too, by app, model and user, priced at list prices (of the apps you may see); give ai for that alone. Use it when something is failing, erroring or slow. raw gives the rows themselves instead: requests by the hour, spans, logs, metrics and errors as JSON, each up to limit, and traces, the newest traces with a span of service, name, status and min_ms over the whole range, each with all its spans. Never holds a call's arguments or answer; a real failure keeps what it said, cleaned of anything that names someone or opens something. Errors are what broke: a caller's mistake (refused, invalid input, no such thing) is status unset, listed only with status unset. What's newest takes up to a minute or two to show.",
                "input": { "type": "object", "properties": {
                    "service": { "type": "string", "description": "only this service: an app's name, or the network's own program" },
                    "trace_id": { "type": "string", "description": "one trace by its id, as its spans: a failure says its trace-id" },
                    "raw": { "type": "boolean", "description": "the rows themselves, as JSON" },
                    "ai": { "type": "boolean", "description": "only what AI Gateway's calls spent: by app, model and user, priced" },
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
            })],
    }
}
