//! The core's own tools: add's and remove's, the Vault's, Backups', Groups', Devices', Widgets', Push's and logs', each defined beside its
//! feature's others and listed here, the one list. Agents use them as they use apps' tools, by the same kind of name
//! (`vault_list`, `logs`), on the command line and over MCP; but they're not apps, so no list of apps has them. Every
//! client lists and runs the same ones: who may do what is the user's rules, and the device's keys. Most are one
//! request, which a client asks its node as its user, or answers itself when it changes who can do what
//! (`settings::answered`), signing that as its user's device; so the remote MCP server's devices answer those
//! themselves (cloudflare/node/src/clients.rs). A tool that is its own route (`Runs::Route`) is defined once, beside
//! what serves it: `POST /v1/<feature>/<tool>` takes its input as its body and answers its output, and `api::tools`
//! mounts every one. The rest still wrap a route defined elsewhere (`Runs::Asks`), or take several requests, or what's
//! on the machine the command line runs on (`Runs::Steps`).

use anyhow::Result;
use serde_json::{Map, Value, json};

pub mod apps;
pub mod backups;
pub mod devices;
pub mod groups;
pub mod push;
pub mod widgets;

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

    /// Its name on a call, which the node keeps with who used each app.
    pub fn client(self) -> &'static str {
        match self {
            Reader::Cli => "cli",
            Reader::Mcp => "mcp",
            Reader::Code => "code",
        }
    }
}

/// Who runs a core tool, and how.
pub struct Asked<'a> {
    pub agent: Option<&'a str>,
    pub reader: Reader,
    /// The user agreed to what it sends or can't undo (`confirm: true`, `--confirm`).
    pub confirmed: bool,
}

/// A request of a node's API.
pub struct Request {
    pub method: &'static str,
    /// With its query, if any.
    pub path: String,
    pub body: Option<Value>,
}

impl Request {
    pub fn get(path: impl Into<String>) -> Request {
        Request { method: "GET", path: path.into(), body: None }
    }

    pub fn post(path: impl Into<String>, body: Value) -> Request {
        Request { method: "POST", path: path.into(), body: Some(body) }
    }
}

/// How a tool runs.
pub enum Runs {
    /// One request, made from its input, and what it says once that's answered.
    Asks { request: fn(&Map<String, Value>, &Asked) -> Result<Request>, said: fn(&Map<String, Value>, Value) -> Result<Value> },
    /// Several, or what's on the machine the command line runs on: its feature's `steps`.
    Steps,
    /// Its own route, `Feature::path`: its input is the request's body, its output the answer.
    Route {
        #[cfg(feature = "node")]
        serves: Serves,
    },
}

/// What serves a tool that is its own route, on a node: as the caller, with the input its schema let through. Its answer
/// is held to its `output` schema. `serves!` makes one.
#[cfg(feature = "node")]
pub type Serves = fn(crate::node::Node, crate::node::Caller, Map<String, Value>) -> futures_util::future::BoxFuture<'static, Result<Value>>;

#[cfg(not(feature = "node"))]
pub type Serves = ();

/// A `Serves` from its `async fn`, which builds without the node don't have.
#[cfg(feature = "node")]
macro_rules! serves {
    ($f:path) => {
        |node, me, input| Box::pin($f(node, me, input))
    };
}
#[cfg(not(feature = "node"))]
macro_rules! serves {
    ($f:path) => {
        ()
    };
}
pub(crate) use serves;

pub struct Tool {
    /// As an app lists a tool (`GET /v1/apps`): named after `<feature>_`, or as the feature, for its one tool with a
    /// command of its own (`logs`).
    pub spec: Value,
    pub runs: Runs,
}

impl Tool {
    pub fn asks(spec: Value, request: fn(&Map<String, Value>, &Asked) -> Result<Request>, said: fn(&Map<String, Value>, Value) -> Result<Value>) -> Tool {
        Tool { spec, runs: Runs::Asks { request, said } }
    }

    pub fn steps(spec: Value) -> Tool {
        Tool { spec, runs: Runs::Steps }
    }

    /// A tool that is its own route: `spec` is everything the command line, MCP and search read of it (`name`, `title`,
    /// `description`, `input`, `reads`, `confirm`, `words`), and `output`, JSON Schema of its answer.
    #[cfg(feature = "node")]
    pub fn route(spec: Value, serves: Serves) -> Tool {
        Tool { spec, runs: Runs::Route { serves } }
    }

    #[cfg(not(feature = "node"))]
    pub fn route(spec: Value, _: Serves) -> Tool {
        Tool { spec, runs: Runs::Route {} }
    }
}

/// Runs a feature's tool that takes several steps (`Runs::Steps`), by its name in the feature, on a client that asks a
/// node.
#[cfg(feature = "node")]
pub type Steps = fn(tool: &str, input: &Map<String, Value>, asked: &Asked, node: &crate::api::Client) -> Result<Value>;

/// One feature of the core, and its tools.
pub struct Feature {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub guide: String,
    pub tools: Vec<Tool>,
    #[cfg(feature = "node")]
    pub steps: Steps,
}

impl Feature {
    /// Its tools as the command line, MCP, search and skills read a set of tools, in the shape an app lists them with:
    /// those named `<name>_<tool>`, since one named as the feature is its own command. They run on the client
    /// (`runs`), never as a call on a device.
    pub fn listed(&self) -> Value {
        let tools: Vec<&Value> = self.tools.iter().map(|t| &t.spec).filter(|t| t["name"] != self.name).collect();
        json!({ "name": self.name, "title": self.title, "description": self.description, "instructions": self.guide, "tools": tools, "runs": "client" })
    }

    /// The route of a tool that is one (`Runs::Route`).
    pub fn path(&self, tool: &Value) -> String {
        format!("/v1/{}/{}", self.name, tool["name"].as_str().unwrap_or(""))
    }

    /// The one request its tool at `at` makes of this input, if it's one: asked by a client of its node, or answered by the client itself.
    pub fn request(&self, at: usize, input: &Map<String, Value>, asked: &Asked) -> Option<Result<Request>> {
        let tool = &self.tools[at];
        match tool.runs {
            Runs::Asks { request, .. } => Some(request(input, asked)),
            Runs::Route { .. } => Some(Ok(Request::post(self.path(&tool.spec), Value::Object(input.clone())))),
            Runs::Steps => None,
        }
    }

    /// What its tool at `at` says, from the answer to its one request: a route's `said`, the words it has for whoever
    /// reads it, or else all it answered.
    pub fn said(&self, at: usize, input: &Map<String, Value>, mut answer: Value) -> Result<Value> {
        match self.tools[at].runs {
            Runs::Asks { said, .. } => said(input, answer),
            _ => Ok(match answer.get_mut("said") {
                Some(said) => said.take(),
                None => answer,
            }),
        }
    }

    /// A tool of its as MCP names it: `vault_list`, or `logs` for the one named as the feature.
    pub fn mcp_name(&self, tool: &Value) -> String {
        match tool["name"].as_str().unwrap_or("") {
            t if t == self.name => t.to_string(),
            t => format!("{}_{t}", self.name),
        }
    }
}

/// Every feature's, in the order they're listed.
pub fn all() -> Vec<Feature> {
    vec![apps::add_feature(), apps::remove_feature(), crate::vault::tools::feature(), backups::feature(), groups::feature(), devices::feature(), widgets::feature(), push::feature(), crate::logs::feature()]
}

/// The feature with the tool `name` (as MCP names it), and the tool's place in it.
pub fn find(name: &str) -> Option<(Feature, usize)> {
    all().into_iter().find_map(|f| {
        let at = f.tools.iter().position(|t| f.mcp_name(&t.spec) == name)?;
        Some((f, at))
    })
}

/// A string input, or what to give.
pub fn text<'a>(input: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    input.get(key).and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("give {key}"))
}

/// What an answer gives, as it is.
pub fn as_given(_: &Map<String, Value>, answer: Value) -> Result<Value> {
    Ok(answer)
}

/// What an answer's `said` field says.
pub fn its_said(_: &Map<String, Value>, answer: Value) -> Result<Value> {
    Ok(answer["said"].clone())
}

#[cfg(feature = "node")]
mod run;
#[cfg(feature = "node")]
pub use run::{and_apps, listed, mcp_tools, no_steps, say_time};
