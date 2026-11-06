//! The core's tools that change who can do what and that someone's device elsewhere may use: a device answers their
//! requests itself, signing what they change as its person's (src/settings.rs), as a machine with a node does for its
//! own programs. Each is the request it makes and what it says once that's answered: the command line asks
//! it through its device (core_tools), and the remote MCP server's devices answer an MCP client's call of one with no
//! node (cloudflare/node/src/clients.rs).

use serde_json::{Map, Value, json};

pub struct Tool {
    /// As MCP names it.
    pub name: &'static str,
    /// The request its input makes: a path and its JSON body, or what's wrong with the input.
    pub asks: fn(&Map<String, Value>) -> Result<(&'static str, Value), String>,
    /// What it says once that request was answered.
    pub said: fn(&Map<String, Value>, &Value) -> Value,
}

pub const HANDLE: Tool = Tool { name: "groups_handle", asks: handle, said: handle_said };

pub const ALL: &[Tool] = &[HANDLE];

pub fn find(name: &str) -> Option<&'static Tool> {
    ALL.iter().find(|t| t.name == name)
}

fn handle(input: &Map<String, Value>) -> Result<(&'static str, Value), String> {
    let handle = input.get("handle").and_then(Value::as_str).ok_or("give handle")?;
    let group = match input.get("group") {
        None | Some(Value::Null) => None,
        Some(Value::String(g)) => Some(g),
        Some(_) => return Err("group is a group's name or id, from groups_list".into()),
    };
    Ok(("/v1/handle", json!({ "handle": handle, "group": group })))
}

fn handle_said(input: &Map<String, Value>, _: &Value) -> Value {
    let handle = input.get("handle").and_then(Value::as_str).unwrap_or("");
    json!(match input.get("group").and_then(Value::as_str) {
        Some(group) => format!("{group}'s handle is now {handle}."),
        None => format!("Your handle is now {handle}."),
    })
}
