//! What serves Insights' tool (insights/src/tools.rs).

use anyhow::Result;
use api::Caller;
use api::runtime::prelude::*;
use api::serves;
use insights::{Ask, cost, view};
use serde_json::{Map, Value, json};

/// Its one tool, and what serves it.
pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![("insights", serves!(insights))]
}

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

async fn insights(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
    let raw = input.get("raw").and_then(Value::as_bool).unwrap_or(false);
    let trace = text("trace_id");
    let effect = text("effect_of");
    let now = custom_build::now_ms();
    let since = match text("since").map(|s| ::device_logs::since(&s)).transpose()? {
        Some(d) => Some(now.saturating_sub(d.as_millis() as u64)),
        None if raw || trace.is_some() => None,
        None if effect.is_some() => Some(now.saturating_sub(7 * DAY)),
        None => Some(now.saturating_sub(DAY)),
    };
    let service = text("service");
    let limit = input.get("limit").and_then(Value::as_u64).map(|n| n as usize);
    if input.get("check").and_then(Value::as_bool).unwrap_or(false) {
        crate::check_telemetry(&rt).await;
    }
    let ask = Ask { queries: queries(raw, input.get("parts").and_then(Value::as_array), trace.is_some(), service.is_some(), effect.is_some()), service, name: text("name"), trace: trace.clone(), since, status: text("status"), least: input.get("min_ms").and_then(Value::as_u64), limit: if raw { limit.unwrap_or(100) } else { ALL_ROWS } };
    let overview = !raw && trace.is_none() && effect.is_none() && ask.service.is_none();
    let ai = input.get("ai").and_then(Value::as_bool).unwrap_or(false);
    // What AI Gateway's calls spent, from its token metric, of every service the viewer may read.
    let spent = Ask { queries: vec!["metrics"], name: Some(insights::GEN_AI_TOKENS.name.to_string()), since, limit: ALL_ROWS, ..Default::default() };
    let range = view::since_said(since.unwrap_or(0), now);
    if ai {
        let rows = crate::insights(&rt, &me, &spent).await?;
        let metrics = rows["metrics"].as_array().cloned().unwrap_or_default();
        return Ok(json!(cost::ai_said(&metrics, &range, |u| crate::user_named(&rt, u)).unwrap_or_else(|| format!("No AI calls recorded in {range}."))));
    }
    // What the network's use of Cloudflare cost, beside the overview: an admin's only, as a service no one else may read.
    let usage = Ask { queries: vec!["metrics"], service: Some(cost::CLOUDFLARE.to_string()), name: None, trace: None, since, status: None, least: None, limit: ALL_ROWS };
    let (rows, usage, spent) = tokio::join!(crate::insights(&rt, &me, &ask), async { if overview { crate::insights(&rt, &me, &usage).await.ok() } else { None } }, async { if overview { crate::insights(&rt, &me, &spent).await.ok() } else { None } });
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
            let cost = cost::said(&metrics, &range);
            let spent = spent.as_ref().and_then(|s| s["metrics"].as_array()).and_then(|m| cost::ai_said(m, &range, |u| crate::user_named(&rt, u)));
            [Some(view::overview(&rows, status, since, now, limit)), cost, spent].into_iter().flatten().collect::<Vec<_>>().join("\n\n")
        }
    };
    Ok(json!(view::with_problem(&rows, now, text)))
}
