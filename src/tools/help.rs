//! A tool's `--help`, and a list of tools, from their input schemas.

use ::custom_build::config::CLI;
use serde_json::Value;

use super::input::{kebab, places, required};
use super::{device_about, own_device};

fn kind(prop: &Value) -> String {
    if let Some(choices) = prop["enum"].as_array() {
        return choices.iter().map(|c| c.as_str().map(String::from).unwrap_or_else(|| c.to_string())).collect::<Vec<_>>().join("|");
    }
    if prop["format"] == "file" {
        return "file".into();
    }
    match prop["type"].as_str() {
        Some("array") => format!("{}...", kind(&prop["items"])),
        Some("object") => "json".into(),
        Some(t) => t.into(),
        None => "value".into(),
    }
}

/// A tool as a command, `one <app> <tool> [<topic>] <text> --other <other>`: the inputs its words fill, then its other
/// required inputs as flags.
pub fn usage(app: &str, tool: &Value) -> String {
    let (required, places) = (required(&tool["input"]), places(tool));
    let mut usage = match tool["id"] == app {
        true => format!("{CLI} {app}"),
        false => format!("{CLI} {app} {}", tool["name"].as_str().unwrap_or("")),
    };
    for p in &places {
        usage.push_str(&match required.contains(p) {
            true => format!(" <{}>", kebab(p)),
            false => format!(" [<{}>]", kebab(p)),
        });
    }
    for r in required.iter().filter(|r| !places.contains(r)) {
        usage.push_str(&format!(" --{} <{}>", kebab(r), kebab(r)));
    }
    usage
}

/// The inputs `usage` leaves out, which are optional flags.
pub fn optional(tool: &Value) -> Vec<(String, Value)> {
    let (required, places) = (required(&tool["input"]), places(tool));
    tool["input"]["properties"].as_object().into_iter().flatten().filter(|(k, _)| !required.contains(k) && !places.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// A tool's `--help`; `call`: an app's, run as a call on a device, which takes `--device`, where the core's don't.
pub(super) fn help(app: &Value, tool: &Value, call: bool) -> String {
    let (a, o) = (app["name"].as_str().unwrap_or(""), tool["name"].as_str().unwrap_or(""));
    let schema = &tool["input"];
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    let required = required(schema);
    let mut usage = usage(a, tool);
    if !optional(tool).is_empty() {
        usage.push_str(" [--<input> <value>]...");
    }
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut names: Vec<&String> = required.iter().filter(|r| props.contains_key(*r)).collect();
    names.extend(props.keys().filter(|p| !required.contains(p)));
    for name in names {
        let p = &props[name];
        let file = p["format"] == "file" || p["items"]["format"] == "file";
        let flag = match file {
            true => format!("--file {}=<path>", kebab(name)),
            false if p["type"] == "boolean" => format!("--{}", kebab(name)),
            false => format!("--{} <{}>", kebab(name), kind(p)),
        };
        let mut about = Vec::new();
        if required.contains(name) {
            about.push("required".to_string());
        }
        if p["type"] == "array" {
            about.push("repeat it for each".into());
        }
        if let Some(d) = p["description"].as_str() {
            about.push(d.to_string());
        }
        rows.push((flag, about.join("; ")));
    }
    if call && !own_device(tool) {
        rows.push(("--device <name>".into(), device_about(app).into()));
    }
    if call {
        rows.push(super::scopes::flag());
    }
    rows.push(("--out <path>".into(), "save what it gives back there".into()));
    rows.push(("--json".into(), "print what it gives back as JSON".into()));
    if tool["confirm"] == true {
        rows.push(("--confirm".into(), format!("go ahead without asking: it {}", crate::core_tools::SENDS)));
    }
    let width = rows.iter().map(|(f, _)| f.chars().count()).max().unwrap_or(0).min(32);
    let mut text = format!("Usage: {usage}\n\n{}\n\nInputs:\n", tool["description"].as_str().unwrap_or(""));
    for (flag, about) in rows {
        let pad = width.saturating_sub(flag.chars().count());
        text.push_str(format!("  {flag}{}  {about}", " ".repeat(pad)).trim_end());
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&match (tool["reads"] == true, tool["confirm"] == true) {
        (true, _) => "It only reads.\n".to_string(),
        (_, true) => format!("{} At a terminal, it asks first.\n", crate::core_tools::Reader::Cli.needs_confirm()),
        _ => String::new(),
    });
    if !required.is_empty() {
        text.push_str(&format!("Inputs not given are read as JSON from stdin: {CLI} {a} {o} < input.json\n"));
    }
    text
}

/// How much of a tool's description a list of tools shows; `--help` has the rest.
const LIST_LENGTH: usize = 90;

/// A description's first sentence, at most about `LIST_LENGTH` characters, ending at a word.
fn first_sentence(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    let line = line.find(". ").map_or(line, |i| &line[..=i]);
    if line.chars().count() <= LIST_LENGTH {
        return line.to_string();
    }
    let cut: String = line.chars().take(LIST_LENGTH).collect();
    format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head))
}

/// What a tool does, in a line: its description's first sentence, and whether it needs --confirm.
fn about(tool: &Value) -> String {
    let mark = if tool["confirm"] == true { " (needs --confirm)" } else { "" };
    format!("{}{mark}", first_sentence(tool["description"].as_str().unwrap_or("")))
}

/// The tools of `apps` a user may call, with their set's name.
fn callable(apps: &[Value]) -> impl Iterator<Item = (&str, &Value)> {
    apps.iter().flat_map(|a| a["tools"].as_array().into_iter().flatten().map(move |o| (a["name"].as_str().unwrap_or(""), o)))
}

/// Every tool of `apps` as the command to type and what it does, aligned.
pub fn list(apps: &[Value]) -> Vec<String> {
    let rows: Vec<(String, String)> = callable(apps).map(|(name, o)| (format!("{CLI} {name} {}", o["name"].as_str().unwrap_or("")), about(o))).collect();
    let width = rows.iter().map(|(c, _)| c.chars().count()).max().unwrap_or(0).clamp(28, 36);
    rows.into_iter().map(|(c, d)| format!("  {c:<width$}  {d}")).collect()
}

/// Every tool of `apps` as a command's forms (`Command.help`): its usage and what it does.
pub fn forms(apps: &[Value]) -> Vec<(String, String)> {
    callable(apps).map(|(name, o)| (usage(name, o).trim_start_matches(&format!("{CLI} ")).to_string(), about(o))).collect()
}
