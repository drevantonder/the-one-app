//! Apps' widgets and screens as MCP Apps, so chat clients that support them draw them inline: one `ui://` view, and
//! two tools that fill it, `widgets_show` and `screens_open` (and `screens_icons` and `screens_changes`, which only the
//! view calls). Clients without MCP Apps get text from both.

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
    // @one/ui, the bridge and what web pages draw for screens, as the web client has them (src/screen/).
    let mut view = include_str!("mcp_view.html").to_string();
    for (name, text) in [
        ("ui.css", include_str!("screen/ui/dist/ui.css")),
        ("bridge.js", include_str!("screen/bridge.js")),
        ("host.js", include_str!("screen/host.js")),
        ("ui.js", include_str!("screen/ui/dist/ui.js")),
    ] {
        view = view.replacen(&format!("/*{name}*/"), text, 1);
    }
    // A screen's `one.copy` writes to the clipboard, which the chat's frame lets it only if asked.
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": MIME, "text": view, "_meta": { "ui": { "prefersBorder": true, "permissions": { "clipboardWrite": {} } } } }] }))
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
            "annotations": one::skills::hints(true, false),
            "_meta": ui,
        }),
        json!({
            "name": "screens_open",
            "description": "Open one of an app's screens inline, where the chat can draw it, for the person to use there.",
            "inputSchema": { "type": "object", "required": ["app"], "properties": {
                "app": { "type": "string", "description": "the app's name, like gmail" },
                "screen": { "type": "string", "description": "the screen's name; main by default" },
                "params": { "type": "object", "description": "what the screen is opened with" },
                "scope": { "type": "string", "description": "the page it's opened from: user, group:<id> or global" },
            } },
            "annotations": one::skills::hints(true, false),
            "_meta": ui,
        }),
        // Only the view calls it, for the icons a screen or widget draws.
        json!({
            "name": "screens_icons",
            "description": "For the view that draws screens and widgets in a chat, not for you: icons' paths by their names.",
            "inputSchema": { "type": "object", "required": ["names"], "properties": {
                "names": { "type": "array", "items": { "type": "string" }, "description": "Phosphor icons' names, like git-branch" },
            } },
            "annotations": one::skills::hints(true, false),
            "_meta": { "ui": { "visibility": ["app"] } },
        }),
        // Only the view calls it, again and again while it's on screen, so what it shows keeps up: a chat sends a view
        // nothing it isn't asked for.
        json!({
            "name": "screens_changes",
            "description": "For the view that draws screens and widgets in a chat, not for you: waits up to 25 seconds for a change to these apps' data.",
            "inputSchema": { "type": "object", "required": ["apps"], "properties": {
                "apps": { "type": "array", "items": { "type": "string" }, "description": "the apps shown" },
            } },
            "annotations": one::skills::hints(true, false),
            "_meta": { "ui": { "visibility": ["app"] } },
        }),
    ]
}

/// Answers `widgets_show` and `screens_open`; None for any other tool.
pub fn call(name: &str, input: &Value) -> Option<Value> {
    let result = match name {
        "widgets_show" => widgets(input),
        "screens_open" => screen(input),
        "screens_icons" => icons(input),
        "screens_changes" => changes(input),
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
    // Its title, as the web client names it; and where the web client shows it, for what a chat can't do (give files).
    let apps = get("/v1/apps").unwrap_or_default();
    let found = apps.as_array().into_iter().flatten().find(|a| a["name"] == app);
    let title = found
        .and_then(|a| a["screens"].as_array().into_iter().flatten().find(|s| s["name"] == screen)?["title"].as_str())
        .or_else(|| found.and_then(|a| a["title"].as_str().or(a["name"].as_str())))
        .unwrap_or(app);
    let address = get("/v1/state").ok().and_then(|s| s["network"]["settings"]["address"].as_str().map(|a| a.trim_end_matches('/').to_string()));
    let web = address.map(|a| format!("{a}/app/{}/{}?{}", urlencode(app), urlencode(screen), query.iter().map(|(k, v)| format!("{k}={}", urlencode(v))).collect::<Vec<_>>().join("&")));
    Ok(json!({
        "content": [{ "type": "text", "text": format!("Opened {app}'s {screen} screen for the person.") }],
        "structuredContent": { "app": app, "screen": screen, "scope": input["scope"], "title": title, "web": web, "html": html },
    }))
}

/// The node's next event that changes what the view shows of `apps` (their data, or which apps there are), within 25
/// seconds; none if nothing does by then.
fn changes(input: &Value) -> anyhow::Result<Value> {
    use std::io::BufRead;
    const WAIT: std::time::Duration = std::time::Duration::from_secs(25);
    let apps: Vec<&str> = input["apps"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut events = vec![];
    let resp = to_node(agent().get(format!("http://{}/v1/events", addr()))).config().timeout_global(Some(WAIT)).build().call()?;
    if resp.status() == 204 {
        // A node with none to say (the Cloudflare node, while no machine is on): the view asks again after the wait.
        std::thread::sleep(WAIT);
    } else {
        anyhow::ensure!(resp.status().is_success(), "the node's events didn't open ({})", resp.status());
        // Read beside the wait, which ends it: as a device, its stream has no timeout of its own, and the thread reading
        // it ends at its next line (a keep-alive at most 15 seconds on), once nobody listens.
        let (tx, lines) = std::sync::mpsc::channel();
        let body = resp.into_body().into_reader();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(body).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let until = std::time::Instant::now() + WAIT;
        while let Ok(line) = lines.recv_timeout(until.saturating_duration_since(std::time::Instant::now())) {
            let Some(event) = line.strip_prefix("data:").and_then(|d| serde_json::from_str::<Value>(d.trim()).ok()) else { continue };
            let named = event["apps"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>()).unwrap_or_default();
            if event["type"] == "changed" && (named.is_empty() || named.iter().any(|a| apps.contains(a))) {
                events.push(event);
                break;
            }
        }
    }
    Ok(json!({ "content": [{ "type": "text", "text": format!("{} changes.", events.len()) }], "structuredContent": { "events": events } }))
}

fn icons(input: &Value) -> anyhow::Result<Value> {
    static ALL: std::sync::LazyLock<serde_json::Map<String, Value>> = std::sync::LazyLock::new(|| serde_json::from_slice(one::screen::ICONS).unwrap_or_default());
    let paths: serde_json::Map<String, Value> =
        input["names"].as_array().into_iter().flatten().filter_map(Value::as_str).take(500).map(|n| (n.to_string(), ALL.get(n).cloned().unwrap_or_else(|| "".into()))).collect();
    Ok(json!({ "content": [{ "type": "text", "text": format!("{} icons.", paths.len()) }], "structuredContent": { "paths": paths } }))
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

/// A number as a person writes it: 3, 2.5, not 2.4999999.
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
