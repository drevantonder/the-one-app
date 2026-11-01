//! Finding operations across every app (`<cli> search`, MCP's `search`), with the exact way to call each and its
//! app's guide, so the long guides come with a search instead of the MCP instructions.

use anyhow::Result;
use one::config::CLI;
use one::guide::Mode;
use serde_json::{Value, json};

use crate::{get, ops};

/// How the Vault's tools are explained, as an app's guide is.
const VAULT_GUIDE: &str = "The Vault keeps the person's keys, sign-ins and accounts as items, each a page's (theirs, a group's, everyone's): fields (secret ones never read back), the sites they're sent to, and the apps switched on for it. Never ask the person to paste a secret (an API key, a password, a token) into the chat, and never paste one yourself. To keep one, call vault_request and give them its link: they pick one of their items or type a new one into a sheet on their own device, and you never see it. To use one yourself, ask with app agents, then call vault_fetch with {{<field>}} where it goes: your server fills it in, only for the item's sites, and you get the answer. If someone pastes a secret anyway, keep it with vault_set and don't repeat it. vault_list shows what's there.";

/// Who a search answers: someone at the command line, or an MCP client in one of its modes.
#[derive(Clone, Copy)]
pub enum For {
    Cli,
    Mcp(Mode),
}

/// Every app installed for this user that has operations, as `GET /v1/apps` gives them, with each operation's input as
/// MCP takes it (files explained, `server` and `confirm` where they apply), and the Vault's tools as an app.
pub fn catalog() -> Result<Vec<Value>> {
    let mut apps = vec![json!({
        "name": "vault", "title": "Vault", "description": "Keys, sign-ins and accounts, used without being seen", "instructions": VAULT_GUIDE,
        "ops": crate::vault_tools().into_iter().map(|t| json!({
            "name": t["name"].as_str().unwrap_or("").trim_start_matches("vault_"), "description": t["description"], "input": t["inputSchema"],
            "reads": t["annotations"]["readOnlyHint"] == true, "confirm": t["annotations"]["destructiveHint"] == true,
        })).collect::<Vec<_>>(),
    })];
    for mut app in get("/v1/apps")?.as_array().cloned().unwrap_or_default() {
        let ops: Vec<Value> = app["ops"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|op| !op["name"].as_str().unwrap_or("").starts_with('_'))
            .map(|op| {
                let mut op = op.clone();
                op["input"] = ops::mcp_schema(&app, &op);
                op
            })
            .collect();
        if !ops.is_empty() {
            app["ops"] = json!(ops);
            apps.push(app);
        }
    }
    Ok(apps)
}

/// Words a search ignores.
const FILLER: &[&str] = &[
    "a", "an", "the", "my", "me", "i", "to", "for", "of", "in", "on", "at", "and", "or", "with", "from", "it", "is", "are", "all", "some", "how", "do", "can", "what", "this", "that",
    "please", "using", "use",
];

/// A word as a search compares it: lower case, without a plural's or a verb's ending.
fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    for end in ["ing", "es", "s", "ed"] {
        if w.len() > end.len() + 3
            && let Some(s) = w.strip_suffix(end)
        {
            return s.to_string();
        }
    }
    w
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && !FILLER.contains(&w.to_lowercase().as_str())).map(stem).collect()
}

/// How well an operation matches: its name counts most, then its app's, its description, and its app's guide.
fn score(app: &Value, op: &Value, query: &[String]) -> u32 {
    let name = words(op["name"].as_str().unwrap_or(""));
    let app_name = words(&format!("{} {}", app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or("")));
    let about = words(op["description"].as_str().unwrap_or(""));
    let guide = words(&format!("{} {}", app["description"].as_str().unwrap_or(""), app["instructions"].as_str().unwrap_or("")));
    let has = |list: &[String], w: &str| list.iter().any(|x| x == w || (w.len() > 3 && x.starts_with(w)));
    query.iter().map(|w| if has(&name, w) { 6 } else { 0 } + if has(&app_name, w) { 4 } else { 0 } + if has(&about, w) { 2 } else { 0 } + if has(&guide, w) { 1 } else { 0 }).sum()
}

/// A JSON Schema as a TypeScript type, as code mode's signatures show it.
pub fn ts(schema: &Value) -> String {
    if let Some(choices) = schema["enum"].as_array() {
        return choices.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" | ");
    }
    match schema["type"].as_str() {
        Some("string") => "string".into(),
        Some("integer" | "number") => "number".into(),
        Some("boolean") => "boolean".into(),
        Some("array") => format!("{}[]", match ts(&schema["items"]) { t if t.contains(' ') => format!("({t})"), t => t }),
        Some("object") | None if schema["properties"].is_object() => {
            let required: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            // The required ones first, as they're asked for.
            let props = schema["properties"].as_object().cloned().unwrap_or_default();
            let mut keys: Vec<&String> = required.iter().filter_map(|r| props.get_key_value(*r).map(|(k, _)| k)).collect();
            keys.extend(props.keys().filter(|k| !required.contains(&k.as_str())));
            let props: Vec<String> = keys.iter().map(|k| format!("{k}{}: {}", if required.contains(&k.as_str()) { "" } else { "?" }, ts(&props[*k]))).collect();
            if props.is_empty() { "{}".into() } else { format!("{{ {} }}", props.join(", ")) }
        }
        Some("object") => "object".into(),
        _ => "unknown".into(),
    }
}

/// The exact way to call an operation, for whoever asked.
fn call(app: &Value, op: &Value, how: For) -> String {
    let (a, o) = (app["name"].as_str().unwrap_or(""), op["name"].as_str().unwrap_or(""));
    match how {
        For::Mcp(Mode::Code) => format!("{}({})", path(a, o), ts(&op["input"])),
        For::Mcp(Mode::All) => format!("{a}_{o} {}", ts(&op["input"])),
        For::Cli => {
            let schema = &op["input"];
            let required: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let mut line = format!("{CLI} {a} {o}");
            for r in &required {
                line.push_str(&format!(" <{}>", r.replace('_', "-")));
            }
            for (k, v) in schema["properties"].as_object().into_iter().flatten().filter(|(k, _)| !required.contains(&k.as_str())) {
                match v["type"] == "boolean" {
                    true => line.push_str(&format!(" [--{}]", k.replace('_', "-"))),
                    false => line.push_str(&format!(" [--{} {}]", k.replace('_', "-"), ts(v))),
                }
            }
            line
        }
    }
}

/// An operation's path in a code mode script: `tools.gmail.search`, or `tools["google-calendar"].events`.
pub fn path(app: &str, op: &str) -> String {
    let key = |k: &str| match k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit()) {
        true => format!(".{k}"),
        false => format!("[{k:?}]"),
    };
    format!("tools{}{}", key(app), key(op))
}

/// The operations that best match `query`, best first, each with how to call it and what it does, then the guide of
/// each app they're in.
pub fn search(apps: &[Value], query: &str, limit: usize, how: For) -> String {
    let query = words(query);
    let mut found: Vec<(u32, usize, &Value, &Value)> = Vec::new();
    for (i, app) in apps.iter().enumerate() {
        for op in app["ops"].as_array().into_iter().flatten() {
            let s = score(app, op, &query);
            if s > 0 {
                found.push((s, i, app, op));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.truncate(limit.max(1));
    if found.is_empty() {
        let names: Vec<&str> = apps.iter().filter_map(|a| a["name"].as_str()).collect();
        return format!("Nothing matches. Try other words, or an app's name: {}.", names.join(", "));
    }
    let mut text = String::new();
    let mut shown: Vec<&Value> = Vec::new();
    for (_, _, app, op) in &found {
        let mark = match (op["reads"] == true, op["confirm"] == true) {
            (true, _) => " It only reads.",
            (_, true) => match how {
                For::Cli => " It can't be undone: give --confirm once the person agreed.",
                For::Mcp(_) => " It can't be undone: give confirm: true once the person agreed.",
            },
            _ => "",
        };
        text.push_str(&format!("{}\n  {}{mark}\n\n", call(app, op, how), op["description"].as_str().unwrap_or("").trim()));
        if !shown.iter().any(|a| a["name"] == app["name"]) {
            shown.push(app);
        }
    }
    for app in shown {
        if let Some(i) = app["instructions"].as_str().filter(|i| !i.is_empty()) {
            text.push_str(&format!("## {} ({})\n\n{}\n\n", app["title"].as_str().unwrap_or(""), app["name"].as_str().unwrap_or(""), i.trim()));
        }
    }
    text.trim_end().to_string()
}

/// MCP's `search` tool.
pub fn search_tool(mode: Mode) -> Value {
    let calls = match mode {
        Mode::Code => "the exact call for a script (tools.<app>.<op>({ ... }))",
        Mode::All => "the tool's name and inputs",
    };
    json!({
        "name": "search",
        "description": format!("Find the operations for a task in every app: give a few words, like \"unread email\" or \"post to the board\". Answers the best matches, each with {calls} and what it does, then the guide of each app they're in. Search before guessing, and to learn an app."),
        "inputSchema": { "type": "object", "required": ["query"], "properties": {
            "query": { "type": "string", "description": "a few words for the task, or an app's name" },
            "limit": { "type": "integer", "description": "how many operations, 8 if not given" },
        } },
        "annotations": { "readOnlyHint": true },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vec<Value> {
        vec![
            json!({ "name": "board", "title": "Board", "description": "One-line posts", "instructions": "Call board_read first.", "ops": [
                { "name": "post", "description": "Post one line to a topic.", "input": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" }, "topic": { "type": "string" } } } },
                { "name": "read", "reads": true, "description": "Read a topic.", "input": { "type": "object", "properties": {} } },
            ] }),
            json!({ "name": "gmail", "title": "Gmail", "description": "Your email", "instructions": "Draft, don't send.", "ops": [
                { "name": "search", "reads": true, "description": "Search messages.", "input": { "type": "object", "required": ["query"], "properties": { "query": { "type": "string" }, "max_results": { "type": "integer" } } } },
                { "name": "send", "confirm": true, "description": "Send an email.", "input": { "type": "object", "properties": { "to": { "type": "array", "items": { "type": "string" } } } } },
            ] }),
            json!({ "name": "google-calendar", "title": "Google Calendar", "description": "Events", "ops": [
                { "name": "events", "reads": true, "description": "List events.", "input": { "type": "object", "properties": { "kind": { "enum": ["a", "b"] } } } },
            ] }),
        ]
    }

    #[test]
    fn search_finds_the_op_with_its_call_and_guide() {
        let text = search(&apps(), "search my emails", 3, For::Mcp(Mode::Code));
        assert!(text.starts_with("tools.gmail.search({ query: string, max_results?: number })\n  Search messages. It only reads."), "{text}");
        assert!(text.contains("## Gmail (gmail)\n\nDraft, don't send."), "{text}");
        let text = search(&apps(), "post to the board", 1, For::Mcp(Mode::All));
        assert!(text.starts_with("board_post { text: string, topic?: string }"), "{text}");
        let text = search(&apps(), "board post", 1, For::Cli);
        assert!(text.starts_with("one board post <text> [--topic string]"), "{text}");
        let text = search(&apps(), "calendar events", 1, For::Mcp(Mode::Code));
        assert!(text.starts_with("tools[\"google-calendar\"].events({ kind?: \"a\" | \"b\" })"), "{text}");
        assert!(search(&apps(), "send", 1, For::Mcp(Mode::Code)).contains("confirm: true"));
        assert!(search(&apps(), "zebra", 3, For::Cli).starts_with("Nothing matches"));
    }
}
