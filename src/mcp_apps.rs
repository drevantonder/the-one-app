//! Apps' widgets and screens as MCP Apps, so chat clients that support them draw them inline: one `ui://` view, and
//! two tools that fill it, `widgets_show` and `screens_open`. Clients without MCP Apps get text from both.

use serde_json::{Value, json};

use one::config::CLI;

use crate::{agent, addr, get, to_node};

const MIME: &str = "text/html;profile=mcp-app";

fn view_uri() -> String {
    format!("ui://{CLI}/view")
}

/// The view both tools draw in: a widget's tree as HTML, or an app's screen in a frame of its own.
pub fn resources() -> Vec<Value> {
    vec![json!({
        "uri": view_uri(), "name": "view", "mimeType": MIME,
        "description": "The person's widgets and app screens, drawn inline.",
        "_meta": { "ui": { "prefersBorder": true } },
    })]
}

pub fn read(uri: &str) -> Result<Value, Value> {
    if uri != view_uri() {
        return Err(json!({ "code": -32002, "message": format!("no resource {uri}") }));
    }
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": MIME, "text": include_str!("mcp_view.html").replace("/*widgets.js*/", include_str!("screen/widgets.js")), "_meta": { "ui": { "prefersBorder": true } } }] }))
}

pub fn tools() -> Vec<Value> {
    let ui = json!({ "ui": { "resourceUri": view_uri() } });
    vec![
        json!({
            "name": "widgets_show",
            "description": "Show the person's widgets inline, drawn as their apps draw them (or as text where a chat can't draw them): all of them, one app's, or one. Use it when they ask how things stand at a glance.",
            "inputSchema": { "type": "object", "properties": {
                "app": { "type": "string", "description": "only this app's widgets" },
                "widget": { "type": "string", "description": "only the widget of this name" },
                "scope": { "type": "string", "description": "whose page: user (theirs), group:<id> or global; every page by default" },
            } },
            "annotations": { "readOnlyHint": true },
            "_meta": ui,
        }),
        json!({
            "name": "screens_open",
            "description": "Open one of an app's screens inline, where the chat can draw it, for the person to use there.",
            "inputSchema": { "type": "object", "required": ["app"], "properties": {
                "app": { "type": "string" },
                "screen": { "type": "string", "description": "the screen's name; main by default" },
                "params": { "type": "object", "description": "what the screen is opened with" },
                "scope": { "type": "string", "description": "the page it's opened from: user, group:<id> or global" },
            } },
            "annotations": { "readOnlyHint": true },
            "_meta": ui,
        }),
    ]
}

/// Answers `widgets_show` and `screens_open`; None for any other tool.
pub fn call(name: &str, input: &Value) -> Option<Value> {
    let result = match name {
        "widgets_show" => widgets(input),
        "screens_open" => screen(input),
        _ => return None,
    };
    Some(result.unwrap_or_else(|e| json!({ "isError": true, "content": [{ "type": "text", "text": format!("{e:#}") }] })))
}

fn widgets(input: &Value) -> anyhow::Result<Value> {
    let scope = input["scope"].as_str().map(|s| format!("?scope={s}")).unwrap_or_default();
    let all = get(&format!("/v1/widgets{scope}"))?;
    let wanted: Vec<Value> = all
        .as_array()
        .into_iter()
        .flatten()
        .filter(|w| input["app"].as_str().is_none_or(|a| w["app"] == a) && input["widget"].as_str().is_none_or(|n| w["widget"] == n))
        .cloned()
        .collect();
    let text = if wanted.is_empty() {
        "No widgets match.".to_string()
    } else {
        wanted.iter().map(|w| format!("{}: {}", w["title"].as_str().unwrap_or(""), w["error"].as_str().map(|e| format!("broken ({e})")).unwrap_or_else(|| text_of(&w["root"])))).collect::<Vec<_>>().join("\n")
    };
    Ok(json!({ "content": [{ "type": "text", "text": text }], "structuredContent": { "widgets": wanted } }))
}

fn screen(input: &Value) -> anyhow::Result<Value> {
    let app = input["app"].as_str().unwrap_or_default();
    let screen = input["screen"].as_str().unwrap_or("main");
    let mut url = format!("http://{}/v1/screens/{app}/{screen}", addr());
    let params = if input["params"].is_object() { input["params"].to_string() } else { "{}".into() };
    let mut query = vec![("params", params)];
    if let Some(scope) = input["scope"].as_str() {
        query.push(("scope", scope.to_string()));
    }
    url.push('?');
    url.push_str(&query.iter().map(|(k, v)| format!("{k}={}", urlencode(v))).collect::<Vec<_>>().join("&"));
    let mut resp = to_node(agent().get(&url)).call()?;
    let html = resp.body_mut().read_to_string()?;
    anyhow::ensure!(resp.status().is_success(), "{}", html.trim());
    Ok(json!({
        "content": [{ "type": "text", "text": format!("Opened {app}'s {screen} screen for the person.") }],
        "structuredContent": { "app": app, "screen": screen, "html": html },
    }))
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A widget's tree as a line of text, for a model, or a chat that can't draw it.
fn text_of(node: &Value) -> String {
    let s = |k: &str| node[k].as_str().unwrap_or("").to_string();
    let n = |k: &str| match &node[k] {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    };
    let parts = |sep: &str| node["c"].as_array().into_iter().flatten().map(text_of).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(sep);
    match node["t"].as_str().unwrap_or("") {
        "col" | "list" => parts("; "),
        "row" => parts(" "),
        "text" | "link" => s("text"),
        "stat" => [n("value"), s("unit"), s("label")].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "),
        "gauge" | "progress" => format!("{} {}/{}", s("label"), n("value"), node["max"].as_f64().unwrap_or(1.0)).trim().to_string(),
        "item" => [s("title"), s("subtitle"), s("trailing")].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", "),
        "toggle" => format!("{}: {}", s("label"), if node["on"] == true { "on" } else { "off" }),
        "time" => node["at"].as_f64().map(relative).unwrap_or_default(),
        _ => String::new(),
    }
}

/// A time in milliseconds, said from now: "5 min ago", "in 2 h".
fn relative(ms: f64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0);
    let s = ((now - ms) / 1000.0).round() as i64;
    let a = s.abs();
    let span = if a < 60 { return "now".into() } else if a < 3600 { format!("{} min", a / 60) } else if a < 86400 { format!("{} h", a / 3600) } else { format!("{} days", a / 86400) };
    if s > 0 { format!("{span} ago") } else { format!("in {span}") }
}
