//! What a tool shows an agent over MCP: its schema, its results with files kept on this device, files it's given, and
//! no more than the budget.

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};

use crate::api::Client;

use super::files::{FILE_LIMIT, as_file, mime};
use super::{device_about, own_device, runs_on_devices};

/// What MCP takes inline from an agent, which sends base64 in its own output.
pub const MCP_FILE_LIMIT: usize = 1 << 20;

/// Whether an input schema's property takes a file (`format: "file"`), or a list of them.
fn takes_file(prop: &Value) -> Option<bool> {
    match (prop["format"] == "file", prop["items"]["format"] == "file") {
        (true, _) => Some(false),
        (_, true) => Some(true),
        _ => None,
    }
}

pub const MCP_FILE: &str = "a file: { base64, name, type } (up to 1 MiB), { file } (a file id another tool gave back) or { url } (public)";

/// A tool's input schema as an MCP tool's: files explained, and `confirm` on what can't be undone.
pub fn mcp_schema(app: &Value, tool: &Value) -> Value {
    let mut schema = tool["input"].clone();
    if !schema.is_object() {
        schema = json!({ "type": "object", "properties": {} });
    }
    if !schema["properties"].is_object() {
        // One that lists no inputs takes any.
        schema["properties"] = json!({});
        schema["additionalProperties"] = json!(true);
    }
    for prop in schema["properties"].as_object_mut().into_iter().flat_map(|p| p.values_mut()) {
        if takes_file(prop).is_some() {
            let about = prop["description"].as_str().map(|d| format!("{d}; ")).unwrap_or_default();
            prop["description"] = json!(format!("{about}{MCP_FILE}"));
        }
    }
    if runs_on_devices(app) && !own_device(tool) {
        schema["properties"]["device"] = json!({ "type": "string", "description": device_about(app) });
    }
    // Every app's call runs on a device, which can say what it did.
    schema["properties"]["trace"] = json!({ "type": "boolean", "description": "true also gives what the call did and where, as lines after the answer, or after why it failed: the devices tried, the one that ran it, each wait in ms, where its data came from" });
    if tool["confirm"] == true {
        schema["properties"]["confirm"] = crate::skills::confirm_input();
    }
    schema
}

pub fn mcp_annotations(tool: &Value) -> Value {
    crate::skills::hints(tool["reads"] == true, tool["confirm"] == true)
}

/// Brings an agent's file inputs to what apps take, `{ name, type, size, base64 }`: from base64, a file id (a blob
/// this device can get) or a public URL.
pub fn mcp_files(node: &Client, tool: &Value, input: &mut Map<String, Value>) -> Result<()> {
    let props = tool["input"]["properties"].as_object().cloned().unwrap_or_default();
    for (name, prop) in props {
        let (Some(list), Some(v)) = (takes_file(&prop), input.get_mut(&name)) else { continue };
        match (list, v) {
            (true, Value::Array(items)) => {
                for item in items {
                    *item = mcp_file(node, item).with_context(|| format!("{name}"))?;
                }
            }
            (_, v) => *v = mcp_file(node, v).with_context(|| format!("{name}"))?,
        }
    }
    Ok(())
}

fn mcp_file(node: &Client, v: &Value) -> Result<Value> {
    let name = v["name"].as_str().map(str::to_string);
    let (bytes, ty, name) = if let Some(data) = v["base64"].as_str() {
        let bytes = data_encoding::BASE64.decode(data.trim().as_bytes()).context("base64 that doesn't decode")?;
        ensure!(bytes.len() <= MCP_FILE_LIMIT, "a file given as base64 is at most 1 MiB; give a file id or URL for a bigger one");
        (bytes, v["type"].as_str().or(v["mimeType"].as_str()).map(str::to_string), name)
    } else if let Some(id) = v["file"].as_str().or(v["blob"].as_str()) {
        let blob = node.open(&format!("/v1/blobs/{id}"), None)?;
        ensure!(blob.status == 200, "no file {id} for you");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(blob.body, FILE_LIMIT as u64 + 1), &mut bytes)?;
        ensure!(bytes.len() <= FILE_LIMIT, "file {id} is over {} MiB", FILE_LIMIT >> 20);
        (bytes, v["type"].as_str().map(str::to_string), name)
    } else if let Some(url) = v["url"].as_str() {
        ensure!(url.starts_with("https://") || url.starts_with("http://"), "a file's url is http or https");
        let mut resp = fetch_file(node, url)?;
        ensure!(resp.status().is_success(), "{url} answered {}", resp.status());
        let ty = resp.headers().get("content-type").and_then(|t| t.to_str().ok()).map(|t| t.split(';').next().unwrap_or(t).to_string());
        let named = url.split(['?', '#']).next().and_then(|u| u.rsplit('/').next()).filter(|n| !n.is_empty()).map(str::to_string);
        (resp.body_mut().with_config().limit(FILE_LIMIT as u64).read_to_vec().with_context(|| format!("{url} is over {} MiB", FILE_LIMIT >> 20))?, v["type"].as_str().map(str::to_string).or(ty), name.or(named))
    } else {
        bail!("give a file as {MCP_FILE}");
    };
    let name = name.unwrap_or_default();
    let ty = ty.unwrap_or_else(|| mime(&name).to_string());
    Ok(json!({ "name": name, "type": ty, "size": bytes.len(), "base64": data_encoding::BASE64.encode(&bytes) }))
}

/// Gets a file an agent gave as a URL. For someone signed in elsewhere, only from the internet, each redirect too: this
/// machine's addresses (its local API, which takes this device for its own programs), its network's and its tailnet's
/// are its person's, not theirs.
fn fetch_file(node: &Client, url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    let agent = match node.elsewhere() {
        true => crate::reach::agent(ureq::Agent::config_builder().http_status_as_error(false), crate::reach::public),
        false => crate::api::agent(),
    };
    agent.get(url).call().with_context(|| format!("couldn't get {url}"))
}

/// What an MCP result gives an agent to read, at most: about 10,000 tokens, below where harnesses warn.
pub const BUDGET: usize = 40_000;

/// Cuts a result's text to the budget, ending with a line that says how much isn't shown and how to ask for less: the
/// tool's inputs that narrow it, by name. Images aren't text, and stay.
pub fn within_budget(content: &mut Value, tool: Option<&Value>) {
    let Some(items) = content.as_array_mut() else { return };
    let total: usize = items.iter().filter_map(|c| c["text"].as_str()).map(|t| t.chars().count()).sum();
    if total <= BUDGET {
        return;
    }
    let mut left = BUDGET;
    for c in items.iter_mut() {
        let Some(text) = c["text"].as_str() else { continue };
        let n = text.chars().count();
        if n <= left {
            left -= n;
            continue;
        }
        let mut cut: String = text.chars().take(left).collect();
        // At the end of a line, when one ends near the cut.
        if let Some(i) = cut.rfind('\n').filter(|i| cut.len() - i < 2_000) {
            cut.truncate(i);
        }
        left = 0;
        c["text"] = json!(cut);
    }
    let shown: usize = items.iter().filter_map(|c| c["text"].as_str()).map(|t| t.chars().count()).sum();
    let narrows = ["limit", "query", "q", "search", "filter", "fields", "since", "after", "before", "start", "end", "range", "topic", "path"];
    let names: Vec<&str> = tool.and_then(|o| o["input"]["properties"].as_object()).into_iter().flatten().map(|(k, _)| k.as_str()).filter(|k| narrows.iter().any(|n| k.eq_ignore_ascii_case(n) || k.to_lowercase().starts_with(&format!("{n}_")) || k.to_lowercase().starts_with("max"))).collect();
    let ask = match names.as_slice() {
        [] => "Ask for less with a narrower call.".to_string(),
        [one] => format!("Ask for less with {one}."),
        [rest @ .., last] => format!("Ask for less with {} or {last}.", rest.join(", ")),
    };
    let line = format!("\n\n[Shown: the first {} of {} characters. {ask}]", group(shown), group(total));
    match items.iter_mut().rev().find(|c| c["text"].is_string()) {
        Some(c) => c["text"] = json!(format!("{}{line}", c["text"].as_str().unwrap_or(""))),
        None => items.push(json!({ "type": "text", "text": line.trim_start() })),
    }
}

/// A count as a person writes it: 40,000.
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A result as MCP content: text, or JSON with each file in it kept as a file this device serves, given back as a
/// reference (name, type, size, its id and a link), with images also shown.
pub fn mcp_content(node: &Client, ok: &Value) -> Value {
    if ok["content"].is_array() {
        return ok["content"].clone();
    }
    let mut shown = Vec::new();
    let mut ok = ok.clone();
    keep_files(node, &mut ok, &mut shown, 0);
    no_pictures(&mut ok);
    let text = match &ok {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let mut content = vec![json!({ "type": "text", "text": text })];
    content.extend(shown);
    json!(content)
}

/// Takes pictures given as `data:` URLs out of a result, to any depth: an agent reads them as a wall of base64, and
/// screens, which draw them, ask the node themselves.
fn no_pictures(v: &mut Value) {
    match v {
        Value::Array(items) => {
            items.retain(|i| !i.as_str().is_some_and(|s| s.starts_with("data:image/")));
            items.iter_mut().for_each(no_pictures);
        }
        Value::Object(map) => {
            map.retain(|_, i| !i.as_str().is_some_and(|s| s.starts_with("data:image/")));
            map.values_mut().for_each(no_pictures);
        }
        _ => {}
    }
}

/// Replaces each file in `v` (to a few levels deep) with a reference to a blob that holds it.
fn keep_files(node: &Client, v: &mut Value, shown: &mut Vec<Value>, depth: usize) {
    if let Some((name, ty, bytes)) = as_file(v) {
        if let Ok(id) = node.put_blob(&bytes) {
            if ty.starts_with("image/") && bytes.len() <= MCP_FILE_LIMIT {
                shown.push(json!({ "type": "image", "data": data_encoding::BASE64.encode(&bytes), "mimeType": ty }));
            }
            // No link to it: a browser can't prove to the node it's one of this machine's own programs (src/api/local.rs);
            // `file get` saves it.
            *v = json!({ "name": name, "type": ty, "size": bytes.len(), "file": id });
        }
        return;
    }
    if depth >= 4 {
        return;
    }
    match v {
        Value::Array(items) => items.iter_mut().for_each(|i| keep_files(node, i, shown, depth + 1)),
        Value::Object(map) => map.values_mut().for_each(|i| keep_files(node, i, shown, depth + 1)),
        _ => {}
    }
}
