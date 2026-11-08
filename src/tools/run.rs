//! Running a tool, the one way every caller does: the command line, MCP and code mode scripts. The core's run here, on
//! the client, asking its node; an app's run as a call, on a device.

use anyhow::bail;
use serde_json::{Map, Value, json};

use crate::api::Client;
use crate::api::client::Called;
use crate::cli::Failed;
use crate::config::CLI;
use crate::core_tools::Reader;

impl Reader {
    /// A tool as this reader calls it: `one gmail sign_in`, `gmail_sign_in`, `tools.gmail.sign_in`; a core tool named as
    /// its feature (`logs`, `remove`) has no tool part: `one remove`, `remove`, `tools.remove`.
    pub fn named(self, app: &str, tool: &str) -> String {
        if tool.is_empty() {
            return match self {
                Reader::Cli => format!("{CLI} {app}"),
                Reader::Mcp => app.to_string(),
                Reader::Code => format!("tools.{app}"),
            };
        }
        match self {
            Reader::Cli => format!("{CLI} {app} {tool}"),
            Reader::Mcp => format!("{app}_{tool}"),
            Reader::Code => crate::skills::path(app, tool),
        }
    }
}

/// How a tool is run, besides its input.
pub struct Run<'a> {
    pub reader: Reader,
    /// The device it runs on, when it's given one.
    pub device: Option<String>,
    pub scopes: Option<Vec<String>>,
    /// The agent credited, if one asked.
    pub agent: Option<&'a str>,
    /// The person agreed to what it sends or can't undo (`confirm: true`, `--confirm`).
    pub confirmed: bool,
    pub trace: bool,
}

/// Runs a tool as MCP or a script gives it (`gmail_search` and its input), checked against its input as MCP lists it:
/// a wrong input says what's wrong, never runs as something else. `spec` is the tool and its set, when the caller has them.
pub fn dispatch(node: &Client, name: &str, mut input: Map<String, Value>, reader: Reader, agent: Option<&str>, spec: Option<(Value, Value)>) -> Called {
    let checked = (|| {
        let found = spec.or_else(|| crate::core_tools::find(name).map(|(f, at)| (f.listed(), f.tools[at].spec.clone()))).or_else(|| name.split_once('_').and_then(|(a, o)| super::find(node, a, o)));
        let Some((set, tool)) = found else { bail!("{}", unknown_tool(node, name)?) };
        super::check(&super::mcp_schema(&set, &tool), &Value::Object(input.clone()))?;
        let device = if super::own_device(&tool) { None } else { input.remove("device").and_then(|d| d.as_str().map(str::to_string)) };
        let scopes = match input.remove("scopes") {
            Some(Value::Array(names)) => Some(super::scopes::read(names.iter().filter_map(Value::as_str))?),
            _ => None,
        };
        let confirmed = input.remove("confirm") == Some(json!(true));
        let trace = input.remove("trace") == Some(json!(true));
        super::mcp_files(node, &tool, &mut input)?;
        Ok((tool["confirm"] == true, Run { reader, device, scopes, agent, confirmed, trace }))
    })();
    match checked {
        Ok((confirm, how)) => run(node, name, confirm, input, how),
        Err(err) => Called { ok: Err(err), trace: None },
    }
}

/// Runs the tool `name` (as MCP names it) as the person: the core's (`core_tools`), else an app's, as a call. What
/// sends or can't be undone (`confirm`) runs only once they agreed.
pub fn run(node: &Client, name: &str, confirm: bool, input: Map<String, Value>, how: Run) -> Called {
    let refused = |why: String| Called { ok: Err(Failed::Refused(why).into()), trace: None };
    let (app, tool) = name.split_once('_').unwrap_or((name, ""));
    if confirm && !how.confirmed {
        return refused(how.reader.unconfirmed(&how.reader.named(app, tool)));
    }
    if let Some((feature, at)) = crate::core_tools::find(name) {
        let ok = feature.run(at, &input, &crate::core_tools::Asked { agent: how.agent, reader: how.reader, confirmed: how.confirmed }, node);
        return Called { ok, trace: None };
    }
    node.call(&json!({ "device": how.device, "scopes": how.scopes, "app": app, "tool": tool, "input": input, "agent": how.agent, "client": how.reader.client() }), how.trace)
}

/// What to say for a tool nobody here lists: an app's tools when the app is right, else the nearest tools.
fn unknown_tool(node: &Client, name: &str) -> anyhow::Result<String> {
    let sets = crate::core_tools::and_apps(node)?;
    if let Some((app, tool)) = name.split_once('_')
        && sets.iter().any(|a| a["name"] == app)
    {
        return super::unknown(node, app, Some(tool), None);
    }
    let names: Vec<String> = sets.iter().flat_map(|a| a["tools"].as_array().into_iter().flatten().filter_map(move |o| Some(format!("{}_{}", a["name"].as_str()?, o["name"].as_str()?)))).collect();
    match super::nearest(name, &names.iter().map(String::as_str).collect::<Vec<_>>()).as_slice() {
        [] => match name.split_once('_') {
            Some((app, tool)) => super::unknown(node, app, Some(tool), None),
            None => Ok(format!("no tool {name}: search finds the tools for a task")),
        },
        near => Ok(format!("no tool {name}: did you mean {}?", near.join(" or "))),
    }
}
