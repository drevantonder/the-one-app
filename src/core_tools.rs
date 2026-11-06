//! The core's own tools: the Vault's, Backups', Groups', Devices', Widgets', Push's and logs', each defined beside its
//! feature and listed here. Agents use them as they use apps' tools, by the same kind of name (`vault_list`, `logs`), on
//! the command line and over MCP; but they're not apps, so no list of apps has them. The client runs each against the
//! API of the node it asks (`api::Client`), as the person, never on another device.

use anyhow::Result;
use serde_json::{Map, Value, json};

/// Who runs a core tool, and how.
pub struct Asked<'a> {
    pub node: &'a crate::api::Client,
    pub agent: Option<&'a str>,
    pub reader: crate::tools::Reader,
    /// The person agreed to what it sends or can't undo (`confirm: true`, `--confirm`).
    pub confirmed: bool,
}

/// One feature of the core, and its tools.
pub struct Feature {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub guide: String,
    /// Its tools as an app lists them (`GET /v1/apps`): each named after `<name>_`, or as the feature, for its one tool
    /// with a command of its own (`logs`).
    pub tools: Vec<Value>,
    /// Whether someone signed in from elsewhere may use it, or only this machine's own person.
    pub elsewhere: bool,
    /// Runs one of its tools, by its name in `tools`.
    pub run: fn(tool: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value>,
}

impl Feature {
    /// Its tools as the command line, MCP, search and skills read a set of tools, in the shape an app lists them with:
    /// those named `<name>_<tool>`, since one named as the feature is its own command. They run here, on the client
    /// (`runs`), never as a call on a device.
    pub fn listed(&self) -> Value {
        let tools: Vec<&Value> = self.tools.iter().filter(|t| t["name"] != self.name).collect();
        json!({ "name": self.name, "title": self.title, "description": self.description, "instructions": self.guide, "tools": tools, "runs": "client" })
    }

    /// A tool of its as MCP names it: `vault_list`, or `logs` for the one named as the feature.
    pub fn mcp_name(&self, tool: &Value) -> String {
        match tool["name"].as_str().unwrap_or("") {
            t if t == self.name => t.to_string(),
            t => format!("{}_{t}", self.name),
        }
    }

    /// Its tools as MCP lists them.
    pub fn mcp_tools(&self) -> Vec<Value> {
        let set = self.listed();
        self.tools.iter().map(|t| crate::tools::mcp_tool(&set, t, self.mcp_name(t))).collect()
    }

    /// Runs its tool `name` (as MCP names it).
    pub fn run(&self, name: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value> {
        let tool = name.strip_prefix(self.name).and_then(|t| t.strip_prefix('_')).unwrap_or(name);
        (self.run)(tool, input, asked)
    }
}

pub fn all() -> Vec<Feature> {
    use crate::node::{backups, devices, groups, push, vault, widgets};
    vec![vault::tools::feature(), backups::tools::feature(), groups::feature(), devices::tools::feature(), widgets::feature(), push::tools::feature(), crate::logs::feature()]
}

/// Those whoever `node` asks for may use: a feature that's this machine's own person's (`elsewhere: false`) isn't for
/// someone signed in from elsewhere.
pub fn usable(node: &crate::api::Client) -> Vec<Feature> {
    all().into_iter().filter(|f| f.elsewhere || !node.elsewhere()).collect()
}

/// The feature with the tool `name` (as MCP names it), and the tool.
pub fn find(name: &str) -> Option<(Feature, Value)> {
    all().into_iter().find_map(|f| {
        let tool = f.tools.iter().find(|t| f.mcp_name(t) == name)?.clone();
        Some((f, tool))
    })
}

/// The core's tools whoever `node` asks for may use, as the command line, search and skills read a set of tools
/// (`Feature::listed`), each feature with any.
pub fn listed(node: &crate::api::Client) -> Vec<Value> {
    usable(node).iter().map(Feature::listed).filter(|f| f["tools"].as_array().is_some_and(|t| !t.is_empty())).collect()
}

/// `listed`, then each app as `GET /v1/apps` gives it.
pub fn and_apps(node: &crate::api::Client) -> Result<Vec<Value>> {
    Ok([listed(node), node.apps()?].concat())
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
