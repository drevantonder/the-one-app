//! A tool's `--help`, and a list of tools, from their input schemas.

use crate::config::CLI;
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

/// A tool's `--help`; `call`: an app's, run as a call on a device, which takes `--device`, where the core's don't.
pub(super) fn help(app: &Value, tool: &Value, call: bool) -> String {
    let (a, o) = (app["name"].as_str().unwrap_or(""), tool["name"].as_str().unwrap_or(""));
    let schema = &tool["input"];
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    let required = required(schema);
    let places = places(tool);
    let mut usage = format!("{CLI} {a} {o}");
    for p in &places {
        usage.push_str(&match required.contains(p) {
            true => format!(" <{}>", kebab(p)),
            false => format!(" [<{}>]", kebab(p)),
        });
    }
    for r in required.iter().filter(|r| !places.contains(r)) {
        usage.push_str(&format!(" --{} <{}>", kebab(r), kebab(r)));
    }
    if props.keys().any(|p| !required.contains(p) && !places.contains(p)) {
        usage.push_str(" [--input value]...");
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
    rows.push(("--out <path>".into(), "save what it gives back there".into()));
    rows.push(("--json".into(), "print what it gives back as JSON".into()));
    if tool["confirm"] == true {
        rows.push(("--confirm".into(), "go ahead without asking: it can't be undone".into()));
    }
    let width = rows.iter().map(|(f, _)| f.chars().count()).max().unwrap_or(0).min(32);
    let mut text = format!("Usage: {usage}\n\n{}\n\nInputs:\n", tool["description"].as_str().unwrap_or(""));
    for (flag, about) in rows {
        let pad = width.saturating_sub(flag.chars().count());
        text.push_str(format!("  {flag}{}  {about}", " ".repeat(pad)).trim_end());
        text.push('\n');
    }
    text.push('\n');
    text.push_str(match (tool["reads"] == true, tool["confirm"] == true) {
        (true, _) => "It only reads.\n",
        (_, true) => "It sends or can't be undone: it asks first, or give --confirm once the person agreed.\n",
        _ => "",
    });
    if !props.is_empty() {
        text.push_str(&format!("Required inputs not given are read as JSON from stdin: {CLI} {a} {o} < input.json; a last word - reads stdin anyway: {CLI} {a} {o} - < input.json\n"));
    }
    text
}

fn first_sentence(text: &str) -> &str {
    let line = text.lines().next().unwrap_or("");
    match line.find(". ") {
        Some(i) => &line[..=i],
        None => line,
    }
}

/// Every tool of `apps` as the command to type and what it does, aligned.
pub fn list(apps: &[Value]) -> Vec<String> {
    let rows: Vec<(String, String)> = apps
        .iter()
        .flat_map(|a| {
            let name = a["name"].as_str().unwrap_or("").to_string();
            a["tools"].as_array().cloned().unwrap_or_default().into_iter().filter(|o| !o["name"].as_str().unwrap_or("").starts_with('_')).map(move |o| {
                let mark = if o["confirm"] == true { " (needs --confirm)" } else { "" };
                (format!("{CLI} {name} {}", o["name"].as_str().unwrap_or("")), format!("{}{mark}", first_sentence(o["description"].as_str().unwrap_or(""))))
            })
        })
        .collect();
    let width = rows.iter().map(|(c, _)| c.chars().count()).max().unwrap_or(0).clamp(28, 36);
    rows.into_iter().map(|(c, d)| format!("  {c:<width$}  {d}")).collect()
}
