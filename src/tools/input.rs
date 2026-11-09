//! A tool's inputs from flags, words, piped JSON and files: `--help` and the schema say which are which.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};

use super::files::read_file;

/// Each flag given, with its values in order: None for one given with no value.
pub type Flags = BTreeMap<String, Vec<Option<String>>>;

/// Splits words from `--flag value`s (or `--flag=value`), as every command takes them. A flag given again adds to a
/// list; `--` ends the flags.
pub fn split(args: &[String]) -> (Vec<String>, Flags) {
    let (mut words, mut flags) = (Vec::new(), Flags::new());
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let key = a.strip_prefix("--").or_else(|| (a == "-h").then_some("h"));
        match key {
            Some("") => {
                // `--` ends the flags: the rest are words.
                words.extend(args[i + 1..].iter().cloned());
                break;
            }
            Some(key) => {
                let (key, value) = match key.split_once('=') {
                    Some((k, v)) => (k, Some(v.to_string())),
                    None => match args.get(i + 1).filter(|v| !v.starts_with("--")) {
                        Some(v) => {
                            i += 1;
                            (key, Some(v.clone()))
                        }
                        None => (key, None),
                    },
                };
                flags.entry(key.to_string()).or_default().push(value);
            }
            None => words.push(a.clone()),
        }
        i += 1;
    }
    (words, flags)
}

pub(super) fn kebab(name: &str) -> String {
    name.replace('_', "-")
}

/// The input a flag names: `--max-results` is `max_results` (or `maxResults`) when the schema has it.
fn input_name(props: &Map<String, Value>, flag: &str) -> String {
    if props.contains_key(flag) {
        return flag.to_string();
    }
    let flat = |s: &str| s.replace(['-', '_'], "").to_lowercase();
    props.keys().find(|p| flat(p) == flat(flag)).cloned().unwrap_or_else(|| flag.replace('-', "_"))
}

/// The inputs words fill, in order: the tool's `words` when it names them, else its required inputs.
pub(super) fn places(tool: &Value) -> Vec<String> {
    match tool["words"].as_array() {
        Some(words) => words.iter().filter_map(Value::as_str).map(String::from).collect(),
        None => required(&tool["input"]),
    }
}


/// Whether a tool's input takes `name`: one of its properties, or anything where it's open-ended. A name it
/// doesn't take (one renamed, say) fails, rather than being passed over and the call run as if it weren't given.
pub fn takes(schema: &Value, name: &str) -> bool {
    schema["properties"].get(name).is_some() || schema["additionalProperties"] == true || !schema["properties"].is_object()
}

/// The tool's input from flags, words (filling `places` in order) and files. Given fewer words than places, the
/// optional places are skipped first: for a tool with a topic and a text, `"hi"` is text and `ideas "hi"` a topic and text.
/// A word left over is an error, never folded into another input: it says to quote text, or which flag to give it as.
pub fn input(schema: &Value, places: &[String], words: Vec<String>, flags: Flags, files: &[String]) -> Result<Map<String, Value>> {
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    let mut input = Map::new();
    for (flag, values) in flags {
        let name = input_name(&props, &flag);
        ensure!(takes(schema, &name), "--{flag} isn't an input of this (see --help)");
        input.insert(name.clone(), value(&flag, &values, props.get(&name))?);
    }
    for f in files {
        let (field, path) = f.split_once('=').with_context(|| format!("--file takes <input>=<path>, like --file attachment=./report.pdf, not {f}"))?;
        let file = read_file(Path::new(path))?;
        let name = input_name(&props, field);
        match props.get(&name).is_some_and(|p| p["type"] == "array") {
            true => input.entry(name).or_insert(json!([])).as_array_mut().context("that input isn't a list")?.push(file),
            false => drop(input.insert(name, file)),
        }
    }
    let required = required(schema);
    let mut open: Vec<&String> = places.iter().filter(|p| !input.contains_key(*p)).collect();
    while open.len() > words.len()
        && let Some(i) = open.iter().position(|p| !required.contains(p))
    {
        open.remove(i);
    }
    let filled = !open.is_empty();
    let mut words = words.into_iter();
    for name in open {
        let Some(word) = words.next() else { break };
        input.insert(name.clone(), value(name, &[Some(word)], props.get(name))?);
    }
    let rest: Vec<String> = words.collect();
    let Some(first) = rest.first() else { return Ok(input) };
    ensure!(
        !filled,
        "{} is left over: words fill {} and nothing more, so put text with spaces in quotes, or give it as a --flag (see --help)",
        rest.join(" "),
        places.iter().map(|p| format!("<{}>", kebab(p))).collect::<Vec<_>>().join(" ")
    );
    let unset: Vec<&String> = props.keys().filter(|p| !input.contains_key(*p) && props[*p]["type"] == "string").collect();
    bail!(
        "{} isn't an input: {} (see --help)",
        rest.join(" "),
        match unset.as_slice() {
            few if rest.len() == 1 && !few.is_empty() && few.len() <= 3 => format!("did you mean {}?", few.iter().map(|p| format!("--{} {first}", kebab(p))).collect::<Vec<_>>().join(" or ")),
            _ => "give inputs as --flags".to_string(),
        }
    );
}

pub(super) fn required(schema: &Value) -> Vec<String> {
    schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

pub(super) fn missing(schema: &Value, input: &Map<String, Value>) -> Vec<String> {
    required(schema).into_iter().filter(|r| !input.contains_key(r)).collect()
}

/// A flag's value as its input's type wants it: text stays text, lists come from repeated flags or a JSON array.
fn value(flag: &str, values: &[Option<String>], prop: Option<&Value>) -> Result<Value> {
    let ty = prop.and_then(|p| p["type"].as_str());
    let one = |v: &Option<String>, ty: Option<&str>| -> Result<Value> {
        let Some(v) = v else {
            ensure!(matches!(ty, Some("boolean") | None), "--{} needs a value", kebab(flag));
            return Ok(json!(true));
        };
        Ok(match ty {
            Some("string") => json!(v),
            Some("integer") => json!(v.parse::<i64>().ok().with_context(|| format!("--{} takes a whole number, not {v}", kebab(flag)))?),
            Some("number") => json!(v.parse::<f64>().ok().with_context(|| format!("--{} takes a number, not {v}", kebab(flag)))?),
            Some("boolean") => match v.as_str() {
                "true" | "yes" | "1" => json!(true),
                "false" | "no" | "0" => json!(false),
                _ => bail!("--{} is true or false, not {v}", kebab(flag)),
            },
            Some("object") => serde_json::from_str(v).ok().filter(Value::is_object).with_context(|| format!("--{} takes a JSON object, like '{{\"a\": 1}}'", kebab(flag)))?,
            // Untyped: JSON if it is, else text.
            _ => serde_json::from_str(v).ok().filter(|v: &Value| !v.is_string()).unwrap_or_else(|| json!(v)),
        })
    };
    match ty {
        Some("array") => {
            let items = prop.and_then(|p| p["items"]["type"].as_str());
            if let [Some(v)] = values
                && v.trim_start().starts_with('[')
                && let Ok(list) = serde_json::from_str::<Value>(v)
            {
                return Ok(list);
            }
            Ok(Value::Array(values.iter().map(|v| one(v, items)).collect::<Result<_>>()?))
        }
        _ => one(values.last().unwrap_or(&None), ty),
    }
}

/// Fills missing inputs from stdin: a user at a terminal types each, a line each; or what's piped, a JSON object of
/// inputs, or the one missing input's text.
pub(super) fn from_stdin(schema: &Value, missing: &[String], input: &mut Map<String, Value>) -> Result<()> {
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    if std::io::stdin().is_terminal() {
        if std::io::stderr().is_terminal() {
            for name in missing {
                eprint!("{}: ", kebab(name));
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                let line = line.trim_end_matches(['\n', '\r']);
                if !line.is_empty() {
                    input.insert(name.clone(), value(name, &[Some(line.to_string())], props.get(name))?);
                }
            }
        }
        return Ok(());
    }
    let Some(text) = piped() else { return Ok(()) };
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(Value::Object(obj)) if missing.len() > 1 || obj.keys().any(|k| props.contains_key(k)) => {
            for (k, v) in obj {
                input.entry(k).or_insert(v);
            }
        }
        _ if missing.len() == 1 => {
            let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
            input.insert(missing[0].clone(), value(&missing[0], &[Some(text)], props.get(&missing[0]))?);
        }
        _ if missing.is_empty() => {}
        _ => bail!("what's piped isn't a JSON object of inputs"),
    }
    Ok(())
}

/// What's piped to stdin, or None when nothing comes: an agent's shell may leave stdin open and empty, so wait only
/// a little for the first byte.
fn piped() -> Option<String> {
    let (first, got_first) = std::sync::mpsc::channel();
    let (all, got_all) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = vec![0u8; 1];
        let n = stdin.read(&mut buf).unwrap_or(0);
        let _ = first.send(n);
        if n == 0 {
            return;
        }
        let _ = stdin.read_to_end(&mut buf);
        let _ = all.send(buf);
    });
    match got_first.recv_timeout(Duration::from_secs(3)) {
        Ok(n) if n > 0 => got_all.recv().ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
        _ => None,
    }
}
