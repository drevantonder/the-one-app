//! A feature of the core, as every client lists it: its tools' specs, which agents use as they use apps' tools, by the
//! same kind of name (`vault_list`, `logs`), on the command line and over MCP. Each is its own route,
//! `POST /api/<feature>/<tool>`, taking its input as its body and answering its output; what serves it is its runtime
//! crate's (`runtime::Runs`). One a client answers itself (`settings::answered`) it signs as its user's device.

use anyhow::Result;
use serde_json::{Map, Value, json};

/// What a tool that needs the user's yes does, wherever it's said.
pub const SENDS: &str = "sends or can't be undone";

/// Who reads what a tool says, and names tools and inputs their own way: someone at the command line, an MCP client, or a
/// code mode script.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reader {
    Cli,
    Mcp,
    Code,
}

impl Reader {
    /// How this reader says the user agreed.
    pub fn confirm(self) -> &'static str {
        match self {
            Reader::Cli => "--confirm",
            Reader::Mcp | Reader::Code => "confirm: true",
        }
    }

    /// What a tool that sends or can't be undone takes, as this reader gives it.
    pub fn needs_confirm(self) -> String {
        format!("It {SENDS}: give {} once the user agreed.", self.confirm())
    }

    /// The refusal of `tool`, which sends or can't be undone, given without the user's yes.
    pub fn unconfirmed(self, tool: &str) -> String {
        format!("{tool} {SENDS}, and the user hasn't agreed to it: ask them, and give {} only if they say yes to this very action", self.confirm())
    }

    /// A tool as this reader calls it: `one gmail sign_in`, `gmail_sign_in`, `tools.gmail.sign_in`; a core tool named as
    /// its feature (`logs`, `remove`) has no tool part: `one remove`, `remove`, `tools.remove`.
    pub fn named(self, app: &str, tool: &str) -> String {
        let cli = custom_build::config::CLI;
        if tool.is_empty() {
            return match self {
                Reader::Cli => format!("{cli} {app}"),
                Reader::Mcp => app.to_string(),
                Reader::Code => format!("tools.{app}"),
            };
        }
        match self {
            Reader::Cli => format!("{cli} {app} {tool}"),
            Reader::Mcp => format!("{app}_{tool}"),
            Reader::Code => code_path(app, tool),
        }
    }

    /// Its name on a call, which the node keeps with who used each app.
    pub fn client(self) -> &'static str {
        match self {
            Reader::Cli => "cli",
            Reader::Mcp => "mcp",
            Reader::Code => "code",
        }
    }
}

/// A tool's path in a code mode script: `tools.gmail.search`, or `tools["google-calendar"].events`.
pub fn code_path(app: &str, tool: &str) -> String {
    let key = |k: &str| match k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit()) {
        true => format!(".{k}"),
        false => format!("[{k:?}]"),
    };
    format!("tools{}{}", key(app), key(tool))
}

/// A request of a device's API: a tool's route, with its input as the body.
pub struct Request {
    pub path: String,
    pub body: Value,
}

/// One feature of the core, and its tools.
pub struct Feature {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub guide: String,
    /// Each tool as an app lists one (`GET /api/apps`): named after `<feature>_`, or as the feature, for its one tool
    /// with a command of its own (`logs`). `name`, `title`, `description`, `input`, `reads`, `confirm`, `words`, and
    /// `output`, JSON Schema of its answer.
    pub tools: Vec<Value>,
}

impl Feature {
    /// Its tools as the command line, MCP, search and skills read a set of tools, in the shape an app lists them with:
    /// those named `<name>_<tool>`, since one named as the feature is its own command. They run on the client
    /// (`runs`), never as a call on a device.
    pub fn listed(&self) -> Value {
        let tools: Vec<&Value> = self.tools.iter().filter(|t| t["name"] != self.name).collect();
        json!({ "name": self.name, "title": self.title, "description": self.description, "instructions": self.guide, "tools": tools, "runs": "client" })
    }

    /// The route of a tool.
    pub fn path(&self, tool: &Value) -> String {
        format!("/api/{}/{}", self.name, tool["name"].as_str().unwrap_or(""))
    }

    /// The request its tool at `at` makes of this input, `confirmed` if the user agreed to what it sends or can't undo.
    pub fn request(&self, at: usize, input: &Map<String, Value>, confirmed: bool) -> Request {
        let tool = &self.tools[at];
        Request { path: self.path(tool), body: body(tool, input.clone(), confirmed) }
    }

    /// A tool of its as MCP names it: `vault_list`, or `logs` for the one named as the feature.
    pub fn mcp_name(&self, tool: &Value) -> String {
        match tool["name"].as_str().unwrap_or("") {
            t if t == self.name => t.to_string(),
            t => format!("{}_{t}", self.name),
        }
    }
}

/// A core tool's route's body: its input, and a tool that takes `confirm` as one (approve: without it, it says whose
/// device it is) gets what the reader said.
pub fn body(tool: &Value, mut input: Map<String, Value>, confirmed: bool) -> Value {
    if tool["input"]["properties"].get("confirm").is_some() {
        input.insert("confirm".into(), json!(confirmed));
    }
    Value::Object(input)
}

/// A string input, or what to give.
pub fn text<'a>(input: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    input.get(key).and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("give {key}"))
}

/// What a tool says: the words of its answer if it has them (`said`), else all it answered.
pub fn said(mut answer: Value) -> Value {
    match answer.get_mut("said") {
        Some(said) => said.take(),
        None => answer,
    }
}

/// A time, for users: today's as its hour, else its date.
#[cfg(not(target_family = "wasm"))]
pub fn say_time(ms: u64) -> String {
    let Ok(t) = jiff::Timestamp::from_millisecond(ms as i64) else { return String::new() };
    let tz = jiff::tz::TimeZone::system();
    let (t, today) = (t.to_zoned(tz.clone()), jiff::Timestamp::now().to_zoned(tz));
    match t.date() == today.date() {
        true => t.strftime("today at %H:%M").to_string(),
        false => t.strftime("%-d %b at %H:%M").to_string(),
    }
}
