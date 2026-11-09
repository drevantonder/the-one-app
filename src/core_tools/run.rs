//! The core's tools as the command line, MCP, search and skills list them, and running one on a client that asks a node.

use anyhow::{Result, bail};
use serde_json::{Map, Value};

use super::{Asked, Feature, all};
use crate::api::Client;

impl Feature {
    /// Its tools as MCP lists them.
    pub fn mcp_tools(&self) -> Vec<Value> {
        let set = self.listed();
        self.tools.iter().map(|t| crate::tools::mcp_tool(&set, &t.spec, self.mcp_name(&t.spec))).collect()
    }

    /// Runs its tool at `at` (`super::find`) as the user, asking `node`.
    pub fn run(&self, at: usize, input: &Map<String, Value>, asked: &Asked, node: &Client) -> Result<Value> {
        match self.request(at, input, asked) {
            Some(request) => {
                let r = request?;
                let answer = match r.body {
                    None => node.get(&r.path)?,
                    Some(body) => node.post_for(asked.agent, &r.path, body)?,
                };
                self.said(at, input, answer)
            }
            None => (self.steps)(self.tools[at].spec["name"].as_str().unwrap_or(""), input, asked, node),
        }
    }
}

/// The steps of a feature whose every tool is one request.
pub fn no_steps(tool: &str, _: &Map<String, Value>, _: &Asked, _: &Client) -> Result<Value> {
    bail!("{tool} is one request, not steps")
}

/// Every feature's tools, as the command line, search and skills read a set of tools (`Feature::listed`), each feature
/// with any.
pub fn listed() -> Vec<Value> {
    all().iter().map(Feature::listed).filter(|f| f["tools"].as_array().is_some_and(|t| !t.is_empty())).collect()
}

/// Every feature's tools as MCP lists them.
pub fn mcp_tools() -> Vec<Value> {
    all().iter().flat_map(Feature::mcp_tools).collect()
}

/// `listed`, then each app as `GET /v1/apps` gives it.
pub fn and_apps(node: &Client) -> Result<Vec<Value>> {
    Ok([listed(), node.apps()?].concat())
}

/// A time, for users: today's as its hour, else its date.
pub fn say_time(ms: u64) -> String {
    let Ok(t) = jiff::Timestamp::from_millisecond(ms as i64) else { return String::new() };
    let tz = jiff::tz::TimeZone::system();
    let (t, today) = (t.to_zoned(tz.clone()), jiff::Timestamp::now().to_zoned(tz));
    match t.date() == today.date() {
        true => t.strftime("today at %H:%M").to_string(),
        false => t.strftime("%-d %b at %H:%M").to_string(),
    }
}
