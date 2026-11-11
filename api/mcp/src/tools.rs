//! Apps' tools from the command line and MCP: inputs from flags, words, piped JSON and files, help built from
//! each tool's input schema, and files in results.

mod help;
mod mcp;
mod run;
pub mod scopes;

pub use help::{forms, help, list, optional, usage};
pub use mcp::{BUDGET, MCP_FILE_LIMIT, mcp_content, mcp_files, mcp_schema, mcp_tool, within_budget};
pub use run::{Run, dispatch, run};

pub use ::api::openapi::{own_device, runs_on_devices};

use anyhow::Result;
use serde_json::Value;

use ::api::Client;
use ::api::tools::nearest;
use ::custom_build::config::CLI;
use ::api::Reader;

/// A tool as the command line reads it, with its set, as the API's document lists them (`Client::sets`); None when it
/// lists none.
pub fn find(node: &Client, app: &str, tool: &str) -> Option<(Value, Value)> {
    let apps = sets(node).ok()?;
    let a = apps.iter().find(|a| a["name"] == app)?.clone();
    let o = a["tools"].as_array()?.iter().find(|o| o["name"] == tool)?.clone();
    Some((a, o))
}

/// A tool by the name MCP calls it, with its set.
pub fn named(node: &Client, name: &str) -> Option<(Value, Value)> {
    node.sets().ok()?.iter().find_map(|s| Some((s.clone(), s["tools"].as_array()?.iter().find(|t| t["id"] == name)?.clone())))
}

/// The sets of tools as commands and search read them: a core tool named as its feature (`logs`) is a command of its own
/// (`one logs`), not one of its set's, and a feature left with none isn't a set.
pub fn sets(node: &Client) -> Result<Vec<Value>> {
    let mut sets = node.sets()?;
    for set in &mut sets {
        let name = set["name"].clone();
        if let Some(tools) = set["tools"].as_array_mut() {
            tools.retain(|t| t["id"] != name);
        }
    }
    sets.retain(|s| s["runs"] != "client" || s["tools"].as_array().is_some_and(|t| !t.is_empty()));
    Ok(sets)
}






/// An empty list of accounts, from a tool that says it lists them (`accounts: true`), in words, with what to do:
/// screens want the list, a user or agent reading wants this.
pub fn none_said(node: &Client, app: &Value, tool: &Value, ok: &Value, reader: Reader) -> Option<String> {
    if tool["accounts"] != true || ok.as_array().is_none_or(|a| !a.is_empty()) {
        return None;
    }
    let (name, title) = (app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or(""));
    let signs_in = app["tools"].as_array().into_iter().flatten().any(|o| o["name"] == "sign_in");
    // One that needs a part on a device, with none of yours running it: signing in can't work yet.
    let parts: Vec<&Value> = app["uses"]["services"].as_array().into_iter().flatten().collect();
    let on_none = || {
        let state = node.state().ok()?;
        Some(!state.network?.devices.iter().any(|d| d.now.services.iter().any(|s| parts.iter().any(|p| *p == s))))
    };
    Some(match (signs_in, app["needs"].as_str()) {
        (_, Some(needs)) if !parts.is_empty() && on_none() == Some(true) => format!("No {title} accounts: {title} isn't on any of your devices yet. {needs}."),
        (true, _) => format!("No account is signed in to {title} yet: {} signs one in.", reader.named(name, "sign_in")),
        (false, Some(needs)) => format!("No {title} accounts: {title} isn't on any of your devices yet. {needs}."),
        (false, None) => format!("No {title} accounts yet."),
    })
}

/// What to say for an app or tool nobody here lists, with the nearest names: `gmail has no tool serch:
/// did you mean search?`, `no app gmial: did you mean gmail?`. On the command line, built-in commands are names too.
/// `commands` are the command line's own commands' forms, when it asks.
pub fn unknown(node: &Client, app: &str, tool: Option<&str>, commands: Option<&[(String, String)]>) -> Result<String> {
    let cli = commands.is_some();
    let apps = sets(node)?;
    let tools_of = |a: &Value| -> Vec<String> { a["tools"].as_array().into_iter().flatten().filter_map(|o| o["name"].as_str()).map(String::from).collect() };
    // One the user could add: the app runtime lists it, without tools for them, since it isn't added where they are.
    let addable = |a: &Value| a["tools"].is_null() && a["deleted"].is_null();
    if let Some(a) = apps.iter().find(|a| a["name"] == app) {
        let title = a["title"].as_str().unwrap_or(app);
        if !a["deleted"].is_null() {
            return Ok(match cli {
                true => format!("{title} was deleted: {CLI} apps restore {app} brings it back, for its owners"),
                false => format!("{title} was deleted: apps_restore with app {app} brings it back, for its owners"),
            });
        }
        if addable(a) {
            return Ok(match cli {
                true => format!("{title} isn't added yet: {CLI} add {app} adds it for you, or {CLI} add {app} --scope global for everyone"),
                false => format!("{title} isn't added yet: add with app {app} and scope user adds it for the user, or global for everyone"),
            });
        }
        let (tool, tools) = (tool.unwrap_or(""), tools_of(a));
        return Ok(match nearest(tool, &tools.iter().map(String::as_str).collect::<Vec<_>>()).first() {
            Some(n) => format!("{app} has no tool {tool}: did you mean {n}?"),
            None => format!("{app} has no tool {tool}. Its tools: {}", tools.join(", ")),
        });
    }
    // A tool's name as a command, one repos_create notes: the command is one repos create notes.
    let as_command = app.match_indices('_').find_map(|(i, _)| {
        let (a, o) = (&app[..i], &app[i + 1..]);
        apps.iter().find(|x| x["name"] == a).is_some_and(|x| tools_of(x).iter().any(|x| x == o)).then(|| format!("{a} {o}"))
    });
    if let Some(command) = as_command.filter(|_| cli) {
        return Ok(format!("that's an MCP tool's name: did you mean {CLI} {command}{}?", tool.map(|o| format!(" {o}")).unwrap_or_default()));
    }
    let names: Vec<String> = apps.iter().filter(|a| a["deleted"].is_null()).filter_map(|a| a["name"].as_str()).map(String::from).collect();
    let forms = commands.unwrap_or_default();
    let commands = ::api::cli::words(forms);
    let all: Vec<&str> = names.iter().chain(commands.iter()).map(String::as_str).collect();
    Ok(match nearest(app, &all).first() {
        Some(n) if *n == app && commands.iter().any(|c| c == n) => format!("usage:\n{}", ::api::cli::help(forms.iter().filter(|(usage, _)| usage.split_whitespace().next() == Some(app))).trim_end()),
        Some(n) if commands.iter().any(|c| c == n) && !names.contains(&n.to_string()) => format!("no command {app}: did you mean {CLI} {n}? {CLI} help says the rest"),
        Some(n) => format!("no app {app}: did you mean {n}?"),
        None if cli => format!("no command or app {app}: {CLI} help says the commands, {CLI} search <words> finds tools"),
        None => format!("no app {app}: search finds the tools for a task"),
    })
}

/// What `device` says it does: which device runs the call, and which when none is given.
fn device_about(app: &Value) -> &'static str {
    match (app["runs"] == "here", runs_on_devices(app)) {
        (true, _) => "which of your devices runs it, by name; all for every one; default is this one",
        (_, true) => "which of your devices runs it, by name; default is the first online one that can, in Device Activity's order",
        _ => "run it on that device (by name)",
    }
}

