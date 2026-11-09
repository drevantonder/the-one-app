//! Apps' widgets and screens as MCP Apps, so chat clients that support them draw them inline: one `ui://` view, and
//! two tools that fill it, `widgets_show` and `screens_open` (and `screens_icons`, `screens_call` and `screens_changes`,
//! which only the view calls). Clients without MCP Apps get text from both.

use serde_json::{Value, json};

use crate::config::CLI;

use crate::api::Client;

const MIME: &str = "text/html;profile=mcp-app";

fn view_uri() -> String {
    format!("ui://{CLI}/view")
}

/// The view both tools draw in: a widget's tree as HTML, or an app's screen in a frame of its own.
pub fn resources() -> Vec<Value> {
    vec![json!({
        "uri": view_uri(), "name": "view", "mimeType": MIME,
        "description": "The user's widgets and app screens, drawn inline.",
        "_meta": { "ui": { "prefersBorder": true } },
    })]
}

pub fn read(uri: &str) -> Result<Value, Value> {
    if uri != view_uri() {
        return Err(json!({ "code": -32002, "message": format!("no resource {uri}") }));
    }
    // @one/ui, the bridge and what web pages draw for screens, as the web client has them (src/screen/).
    let mut view = include_str!("view.html").to_string();
    for (name, text) in [
        ("ui.css", crate::ui_folder::text("src/screen/ui/dist/ui.css", include_str!("../screen/ui/dist/ui.css"))),
        ("bridge.js", crate::ui_folder::text("src/screen/bridge.js", include_str!("../screen/bridge.js"))),
        ("host.js", crate::ui_folder::text("src/screen/host.js", include_str!("../screen/host.js"))),
        ("ui.js", crate::ui_folder::text("src/screen/ui/dist/ui.js", include_str!("../screen/ui/dist/ui.js"))),
    ] {
        view = view.replacen(&format!("/*{name}*/"), &text, 1);
    }
    // A screen's `one.copy` writes to the clipboard, which the chat's frame lets it only if asked.
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": MIME, "text": view, "_meta": { "ui": { "prefersBorder": true, "permissions": { "clipboardWrite": {} } } } }] }))
}

pub fn tools() -> Vec<Value> {
    let ui = json!({ "ui": { "resourceUri": view_uri() } });
    vec![
        {
            let mut show = crate::widgets::tools::feature().mcp_tools().remove(0);
            show["_meta"] = ui.clone();
            show
        },
        json!({
            "name": "screens_open",
            "description": "Open one of an app's screens inline, where the chat can draw it, for the user to use there.",
            "inputSchema": { "type": "object", "required": ["app"], "properties": {
                "app": { "type": "string", "description": "the app's name, like gmail" },
                "screen": { "type": "string", "description": "the screen's name; main by default" },
                "params": { "type": "object", "description": "what the screen is opened with" },
                "scope": { "type": "string", "description": "the page it's opened from: user, group:<id> or global" },
            } },
            "annotations": crate::skills::hints(true, false),
            "_meta": ui,
        }),
        // Only the view calls it, for the icons a screen or widget draws.
        json!({
            "name": "screens_icons",
            "description": "For the view that draws screens and widgets in a chat, not for you: icons' paths by their names.",
            "inputSchema": { "type": "object", "required": ["names"], "properties": {
                "names": { "type": "array", "items": { "type": "string" }, "description": "Phosphor icons' names, like git-branch" },
            } },
            "annotations": crate::skills::hints(true, false),
            "_meta": { "ui": { "visibility": ["app"] } },
        }),
        // Only the view calls it, for each call a screen or widget makes.
        json!({
            "name": "screens_call",
            "description": "For the view that draws screens and widgets in a chat, not for you: runs one of an app's tools for what's on screen.",
            "inputSchema": { "type": "object", "required": ["app", "tool"], "properties": {
                "app": { "type": "string", "description": "the app's name" },
                "tool": { "type": "string", "description": "the tool's name in the app" },
                "input": { "type": "object", "description": "the tool's input" },
                "confirm": { "type": "boolean", "description": "the user pressed it, for a tool that sends or can't be undone" },
            } },
            "annotations": crate::skills::hints(false, false),
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
            "annotations": crate::skills::hints(true, false),
            "_meta": { "ui": { "visibility": ["app"] } },
        }),
    ]
}

/// Answers `widgets_show` and `screens_open`; None for any other tool.
pub fn call(node: &Client, name: &str, input: &Value) -> Option<Value> {
    let result = match name {
        "widgets_show" => shown(node, input),
        "screens_open" => screen(node, input),
        "screens_icons" => icons(input),
        "screens_call" => for_screen(node, input),
        "screens_changes" => changes(node, input),
        _ => return None,
    };
    Some(result.unwrap_or_else(|e| json!({ "isError": true, "content": [{ "type": "text", "text": format!("{e:#}") }] })))
}

/// `widgets_show` as a chat that draws them answers it: with what to draw, and in words for what can't.
fn shown(node: &Client, input: &Value) -> anyhow::Result<Value> {
    let shown = node.post("/v1/widgets/show", input.clone())?;
    Ok(json!({ "content": [{ "type": "text", "text": shown["said"] }], "structuredContent": { "widgets": shown["widgets"] } }))
}

fn screen(node: &Client, input: &Value) -> anyhow::Result<Value> {
    let app = input["app"].as_str().unwrap_or_default();
    let screen = input["screen"].as_str().unwrap_or("main");
    let mut url = format!("/v1/screens/{app}/{screen}");
    let params = if input["params"].is_object() { input["params"].to_string() } else { "{}".into() };
    let mut query = vec![("params", params)];
    if let Some(scope) = input["scope"].as_str() {
        query.push(("scope", scope.to_string()));
    }
    url.push('?');
    url.push_str(&query.iter().map(|(k, v)| format!("{k}={}", urlencode(v))).collect::<Vec<_>>().join("&"));
    let page = node.open(&url, None)?;
    let mut html = String::new();
    std::io::Read::read_to_string(&mut { page.body }, &mut html)?;
    anyhow::ensure!(page.status == 200, "{}", html.trim());
    // Its title, as the web client names it; and where the web client shows it, for what a chat can't do (give files).
    let apps = node.apps().unwrap_or_default();
    let found = apps.iter().find(|a| a["name"] == app);
    let title = found
        .and_then(|a| a["screens"].as_array().into_iter().flatten().find(|s| s["name"] == screen)?["title"].as_str())
        .or_else(|| found.and_then(|a| a["title"].as_str().or(a["name"].as_str())))
        .unwrap_or(app);
    let address = node.state().ok().and_then(|s| s.address().map(|a| a.trim_end_matches('/').to_string()));
    let web = address.map(|a| format!("{a}/app/{}/{}?{}", urlencode(app), urlencode(screen), query.iter().map(|(k, v)| format!("{k}={}", urlencode(v))).collect::<Vec<_>>().join("&")));
    Ok(json!({
        "content": [{ "type": "text", "text": format!("Opened {app}'s {screen} screen for the user.") }],
        "structuredContent": { "app": app, "screen": screen, "scope": input["scope"], "title": title, "web": web, "html": html },
    }))
}

/// A call a screen or widget makes, answered as on every client with the tool's own answer, not what an agent reads of it
/// (cut to a budget, its pictures taken out, an empty list in words). It reaches the tools only screens use, which a chat
/// won't let a view call unless they're listed for it, and what's still refused without `confirm` here is refused
/// unless the view says the user pressed it.
fn for_screen(node: &Client, input: &Value) -> anyhow::Result<Value> {
    let (app, tool) = (input["app"].as_str().unwrap_or_default(), input["tool"].as_str().unwrap_or_default());
    let found = crate::tools::find(node, app, tool);
    let mut arguments = input["input"].as_object().cloned().unwrap_or_default();
    if input["confirm"] == true && found.as_ref().is_some_and(|(_, spec)| spec["confirm"] == true) {
        arguments.insert("confirm".into(), true.into());
    }
    let ok = crate::tools::dispatch(node, &format!("{app}_{tool}"), arguments, crate::core_tools::Reader::Mcp, None, found).ok?;
    Ok(json!({ "content": [{ "type": "text", "text": "Done." }], "structuredContent": { "ok": ok } }))
}

/// The node's next event that changes what the view shows of `apps` (their data, or which apps there are), within 25
/// seconds; none if nothing does by then.
fn changes(node: &Client, input: &Value) -> anyhow::Result<Value> {
    use std::io::BufRead;
    const WAIT: std::time::Duration = std::time::Duration::from_secs(25);
    let apps: Vec<&str> = input["apps"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut events = vec![];
    let resp = node.open("/v1/events", Some(WAIT))?;
    if resp.status == 204 {
        // A node with none to say (the Cloudflare node, while no machine is on): the view asks again after the wait.
        std::thread::sleep(WAIT);
    } else {
        anyhow::ensure!(resp.status == 200, "the node's events didn't open ({})", resp.status);
        // Read beside the wait, which ends it: as a device, its stream has no timeout of its own, and the thread reading
        // it ends at its next line (a keep-alive at most 15 seconds on), once nobody listens.
        let (tx, lines) = std::sync::mpsc::channel();
        let body = resp.body;
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
    static ALL: std::sync::LazyLock<serde_json::Map<String, Value>> = std::sync::LazyLock::new(|| serde_json::from_slice(crate::screen::ICONS).unwrap_or_default());
    let paths: serde_json::Map<String, Value> =
        input["names"].as_array().into_iter().flatten().filter_map(Value::as_str).take(500).map(|n| (n.to_string(), ALL.get(n).cloned().unwrap_or_else(|| "".into()))).collect();
    Ok(json!({ "content": [{ "type": "text", "text": format!("{} icons.", paths.len()) }], "structuredContent": { "paths": paths } }))
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
