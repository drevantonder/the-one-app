//! Running a tool, the one way every caller does: the command line, MCP and code mode scripts. The core's run here, on
//! the client, asking its app runtime; an app's run as a call, on a device.

use anyhow::bail;
use serde_json::{Map, Value, json};

use ::api::Client;
use ::api::client::Called;
use ::api::cli::Failed;
use ::api::Reader;

/// How a tool is run, besides its input.
pub struct Run<'a> {
    pub reader: Reader,
    /// The device it runs on, when it's given one.
    pub device: Option<String>,
    pub scopes: Option<Vec<String>>,
    /// The agent credited, if one asked.
    pub agent: Option<&'a str>,
    /// The user agreed to what it sends or can't undo (`confirm: true`, `--confirm`).
    pub confirmed: bool,
    pub trace: bool,
}

/// Runs a tool as MCP or a script gives it (`gmail_search` and its input), checked against its input as MCP lists it:
/// a wrong input says what's wrong, never runs as something else. `spec` is the tool and its set, when the caller has them.
pub fn dispatch(api: &Client, name: &str, mut input: Map<String, Value>, reader: Reader, agent: Option<&str>, spec: Option<(Value, Value)>) -> Called {
    let checked = (|| {
        let found = spec.or_else(|| super::named(api, name));
        let Some((set, tool)) = found else { bail!("{}", unknown_tool(api, name)?) };
        ::api::tools::check(&super::mcp_schema(&set, &tool), &Value::Object(input.clone()))?;
        let device = if super::own_device(&tool) { None } else { input.remove("device").and_then(|d| d.as_str().map(str::to_string)) };
        let scopes = match input.remove("scopes") {
            Some(Value::Array(names)) => Some(super::scopes::read(names.iter().filter_map(Value::as_str))?),
            _ => None,
        };
        let confirmed = input.remove("confirm") == Some(json!(true));
        let trace = input.remove("trace") == Some(json!(true));
        super::mcp_files(api, &tool, &mut input)?;
        Ok((tool, Run { reader, device, scopes, agent, confirmed, trace }))
    })();
    match checked {
        Ok((tool, how)) => run(api, name, &tool, input, how),
        Err(err) => Called { ok: Err(err), trace: None },
    }
}

/// Runs `tool` (`name` as MCP names it) as the user: at its route if it has one, else as a call of its app. What sends or
/// can't be undone (`confirm`) runs only once they agreed.
pub fn run(api: &Client, name: &str, tool_spec: &Value, input: Map<String, Value>, how: Run) -> Called {
    let refused = |why: String| Called { ok: Err(Failed::Refused(why).into()), trace: None };
    let (app, tool) = name.split_once('_').unwrap_or((name, ""));
    if tool_spec["confirm"] == true && !how.confirmed {
        return refused(how.reader.unconfirmed(&how.reader.named(app, tool)));
    }
    if let Some(route) = tool_spec["route"].as_str() {
        let body = ::api::feature::body(tool_spec, input, how.confirmed);
        let called = api.route(how.agent, route, &body, how.trace);
        return Called { ok: called.ok.map(::api::feature::said), trace: called.trace };
    }
    api.call(&json!({ "device": how.device, "scopes": how.scopes, "app": app, "tool": tool, "input": input, "agent": how.agent, "client": how.reader.client() }), how.trace)
}

/// What to say for a tool nobody here lists: an app's tools when the app is right, else the nearest tools.
fn unknown_tool(api: &Client, name: &str) -> anyhow::Result<String> {
    let sets = super::sets(api)?;
    if let Some((app, tool)) = name.split_once('_')
        && sets.iter().any(|a| a["name"] == app)
    {
        return super::unknown(api, app, Some(tool), None);
    }
    let names: Vec<String> = sets.iter().flat_map(|a| a["tools"].as_array().into_iter().flatten().filter_map(move |o| Some(format!("{}_{}", a["name"].as_str()?, o["name"].as_str()?)))).collect();
    match ::api::tools::nearest(name, &names.iter().map(String::as_str).collect::<Vec<_>>()).as_slice() {
        [] => match name.split_once('_') {
            Some((app, tool)) => super::unknown(api, app, Some(tool), None),
            None => Ok(format!("no tool {name}: search finds the tools for a task")),
        },
        near => Ok(format!("no tool {name}: did you mean {}?", near.join(" or "))),
    }
}
