//! What serves Widgets' tool (widgets/src/tools.rs).

use anyhow::Result;
use api::Caller;
use api::runtime::prelude::*;
use api::serves;
use serde_json::{Map, Value, json};

use crate::draw;

/// What serves each of Widgets' tools (widgets/src/tools.rs).
pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![("show", serves!(show))]
}

/// The widgets `input` asks for (`scope`, `app`, `widget`): in words, and as they are to draw.
async fn show(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let alone = input.get("app").and_then(Value::as_str).zip(input.get("widget").and_then(Value::as_str));
    let all = draw::widgets(&rt, me, input.get("scope").and_then(Value::as_str), alone).await?;
    let wanted: Vec<Value> = all
        .into_iter()
        .filter(|w| input.get("app").and_then(Value::as_str).is_none_or(|a| w["app"] == a) && input.get("widget").and_then(Value::as_str).is_none_or(|n| w["widget"] == n))
        .collect();
    let said = if wanted.is_empty() {
        "No widgets match.".to_string()
    } else {
        wanted.iter().map(|w| format!("{}: {}", w["title"].as_str().unwrap_or(""), w["error"].as_str().map(|e| format!("broken ({e})")).unwrap_or_else(|| text_of(&w["root"])))).collect::<Vec<_>>().join("\n")
    };
    Ok(json!({ "said": said, "widgets": wanted }))
}

/// A widget's tree as a line of text, for a model, or a chat that can't draw it.
fn text_of(node: &Value) -> String {
    let s = |k: &str| node[k].as_str().unwrap_or("").to_string();
    let n = |k: &str| match &node[k] {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    };
    // A gauge, bar or chart's name: its alt, said only to readers (a Devices ring is "RAM"), or the label drawn with it.
    let name = node["alt"].as_str().filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s("label"));
    let values = node["values"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect::<Vec<_>>();
    let parts = |sep: &str| node["c"].as_array().into_iter().flatten().map(text_of).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(sep);
    match node["t"].as_str().unwrap_or("") {
        "col" | "list" => parts("; "),
        "row" => parts(" "),
        "text" | "link" => s("text"),
        "stat" => [n("value"), s("unit"), s("label")].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "),
        "gauge" | "progress" => said(&name, &percent(node["value"].as_f64(), node["max"].as_f64().filter(|m| *m != 0.0).unwrap_or(1.0))),
        "spark" => said(&name, &match (values.last(), node["max"].as_f64()) {
            (Some(v), Some(max)) if max != 0.0 => percent(Some(*v), max),
            (Some(v), _) => number(*v),
            _ => String::new(),
        }),
        "chart" => {
            let labels = node["labels"].as_array().cloned().unwrap_or_default();
            let each = values.iter().enumerate().map(|(i, v)| [labels.get(i).and_then(Value::as_str).unwrap_or(""), &number(*v)].join(" ").trim().to_string());
            said(&name, &each.collect::<Vec<_>>().join(", "))
        }
        "item" => [s("title"), s("subtitle"), s("trailing")].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", "),
        "toggle" => format!("{}: {}", s("label"), if node["on"] == true { "on" } else { "off" }),
        "time" => node["at"].as_f64().map(relative).unwrap_or_default(),
        _ => String::new(),
    }
}

/// What a gauge or chart shows, after its name: "RAM 17%".
fn said(name: &str, value: &str) -> String {
    if value.is_empty() { String::new() } else { format!("{name} {value}").trim().to_string() }
}

/// A value as a share of its max: "17%".
fn percent(value: Option<f64>, max: f64) -> String {
    value.map(|v| format!("{}%", (v / max * 100.0).clamp(0.0, 100.0).round())).unwrap_or_default()
}

/// A number as a user writes it: 3, 2.5, not 2.4999999.
fn number(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

/// A time in milliseconds, said from now: "5 min ago", "in 2 h".
fn relative(ms: f64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0);
    let s = ((now - ms) / 1000.0).round() as i64;
    let a = s.abs();
    let span = if a < 60 { return "now".into() } else if a < 3600 { format!("{} min", a / 60) } else if a < 86400 { format!("{} h", a / 3600) } else { format!("{} days", a / 86400) };
    if s > 0 { format!("{span} ago") } else { format!("in {span}") }
}
