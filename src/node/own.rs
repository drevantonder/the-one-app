//! The node's own apps (the Vault, Backups, Groups, Devices, Widgets): tools that search, scripts, skills, MCP and
//! the command line show as an app's, each defined beside what it does, and run by this machine's CLI and MCP against
//! its node's API (`api::Client`), as the person, never on another device.

use anyhow::Result;
use serde_json::{Map, Value, json};

/// Who runs a node app's tool, and how.
pub struct Asked<'a> {
    pub node: &'a crate::api::Client,
    pub agent: Option<&'a str>,
    /// `cli` or `mcp`.
    pub client: &'a str,
    /// The person agreed to what it sends or can't undo (`confirm: true`, `--confirm`).
    pub confirmed: bool,
}

pub struct NodeApp {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub guide: String,
    /// Its tools as MCP lists them, each named `<app>_<tool>`.
    pub tools: Vec<Value>,
    /// Whether someone signed in from elsewhere may use it, or only this machine's own person.
    pub elsewhere: bool,
    pub run: fn(tool: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value>,
}

impl NodeApp {
    /// As `GET /v1/apps` describes an app, for search, scripts, skills and the command line.
    pub fn listed(&self) -> Value {
        let name = self.name;
        json!({
            "name": name, "title": self.title, "description": self.description, "instructions": self.guide,
            "ops": self.tools.iter().map(|t| {
                let confirm = t["annotations"]["destructiveHint"] == true;
                json!({ "name": t["name"].as_str().unwrap_or("").trim_start_matches(&format!("{name}_")), "description": t["description"], "input": t["inputSchema"], "reads": t["annotations"]["readOnlyHint"] == true, "confirm": confirm })
            }).collect::<Vec<_>>(),
        })
    }
}

pub fn all() -> Vec<NodeApp> {
    vec![super::vault::app::app(), super::backups::app::app(), super::groups::app(), super::devices::app::app(), super::widgets::app()]
}

pub fn find(name: &str) -> Option<NodeApp> {
    all().into_iter().find(|a| a.name == name)
}

pub fn listed() -> Vec<Value> {
    all().iter().map(NodeApp::listed).collect()
}

/// A string input, or what to give.
pub fn text<'a>(input: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    input.get(key).and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("give {key}"))
}

/// A time, for people: today's as its hour, else its date.
pub fn say_time(ms: u64) -> String {
    let Ok(t) = jiff::Timestamp::from_millisecond(ms as i64) else { return String::new() };
    let tz = jiff::tz::TimeZone::system();
    let (t, today) = (t.to_zoned(tz.clone()), jiff::Timestamp::now().to_zoned(tz));
    match t.date() == today.date() {
        true => t.strftime("today at %H:%M").to_string(),
        false => t.strftime("%-d %b at %H:%M").to_string(),
    }
}
