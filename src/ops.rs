//! Apps' operations from the command line and MCP: inputs from flags, words, piped JSON and files, help built from
//! each operation's input schema, and files in results.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use one::config::CLI;
use serde_json::{Map, Value, json};

use crate::{agent_from_env, get, post};

/// Files up to this size go inline, as base64, in an operation's input.
pub const FILE_LIMIT: usize = 10 << 20;
/// What MCP takes inline from an agent, which sends base64 in its own output.
pub const MCP_FILE_LIMIT: usize = 1 << 20;

/// An operation as `GET /v1/apps` describes it, with its app; None when no app or sidecar here lists it.
pub fn find(app: &str, op: &str) -> Option<(Value, Value)> {
    let apps = get("/v1/apps").ok()?;
    let a = apps.as_array()?.iter().find(|a| a["name"] == app)?.clone();
    let o = a["ops"].as_array()?.iter().find(|o| o["name"] == op)?.clone();
    Some((a, o))
}

/// `<CLI> <app> <op> [words] [--input value]...`: runs it and prints what it gives back.
pub fn run(app: &str, op: &str, args: &[String]) -> Result<()> {
    let (words, mut flags) = split(args);
    let spec = find(app, op);
    if flags.contains_key("help") || flags.contains_key("h") {
        let Some((a, o)) = &spec else { bail!("{}", unknown(app, op)) };
        print!("{}", help(a, o));
        return Ok(());
    }
    let mut take = |k: &str| flags.remove(k).and_then(|v| v.into_iter().last().flatten());
    let server = take("server").or_else(|| take("device"));
    let scopes = take("scopes").map(|s| s.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>());
    let out = take("out");
    let confirmed = flags.remove("confirm").is_some() || flags.remove("yes").is_some();
    let files: Vec<String> = flags.remove("file").unwrap_or_default().into_iter().flatten().collect();
    let schema = spec.as_ref().map(|(_, o)| o["input"].clone()).unwrap_or(Value::Null);
    let mut input = input(&schema, words, flags, &files)?;
    let missing = missing(&schema, &input);
    if !missing.is_empty() {
        from_stdin(&schema, &missing, &mut input)?;
        let missing = self::missing(&schema, &input);
        ensure!(
            missing.is_empty(),
            "missing {}: give {} as flags, or pipe the inputs as JSON: {CLI} {app} {op} < input.json. See {CLI} {app} {op} --help.",
            missing.join(", "),
            if missing.len() == 1 { "it" } else { "them" }
        );
    }
    if spec.as_ref().is_some_and(|(_, o)| o["confirm"] == true) && !confirmed {
        ask(&format!("{CLI} {app} {op} can't be undone. Go ahead?"), &format!("{app} {op} can't be undone: run it again with --confirm"))?;
    }
    let r = post("/v1/call", json!({ "server": server, "scopes": scopes, "app": app, "op": op, "input": input, "agent": agent_from_env(), "client": "cli" }))?;
    show(app, op, &r["ok"], out.as_deref())
}

/// What to say for an app or operation nobody here lists.
fn unknown(app: &str, op: &str) -> String {
    match get("/v1/apps").ok().and_then(|a| a.as_array()?.iter().find(|a| a["name"] == app).cloned()) {
        Some(_) => format!("{app} has no operation {op}; {CLI} {app} lists them"),
        None => format!("no app {app}; {CLI} apps lists them"),
    }
}

/// Asks at a terminal; elsewhere (piped, or an agent) it fails with `otherwise`.
fn ask(question: &str, otherwise: &str) -> Result<()> {
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        bail!("{otherwise}");
    }
    eprint!("{question} [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    ensure!(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"), "stopped; nothing changed");
    Ok(())
}

/// Splits words from `--flag value`s. A flag given again adds to a list; a flag with no value is None.
fn split(args: &[String]) -> (Vec<String>, BTreeMap<String, Vec<Option<String>>>) {
    let (mut words, mut flags) = (Vec::new(), BTreeMap::<String, Vec<Option<String>>>::new());
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

fn kebab(name: &str) -> String {
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

/// The operation's input from flags, words (the required inputs, in order) and files.
fn input(schema: &Value, words: Vec<String>, flags: BTreeMap<String, Vec<Option<String>>>, files: &[String]) -> Result<Map<String, Value>> {
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    let mut input = Map::new();
    for (flag, values) in flags {
        let name = input_name(&props, &flag);
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
    let open: Vec<String> = required(schema).into_iter().filter(|r| !input.contains_key(r)).collect();
    let mut words = words.into_iter().peekable();
    for (i, name) in open.iter().enumerate() {
        let Some(word) = words.next() else { break };
        let prop = props.get(name);
        // The last word input takes the rest when it's text: `one board post Starting the CLI work`.
        let word = match i + 1 == open.len() && prop.is_some_and(|p| p["type"] == "string") && words.peek().is_some() {
            true => std::iter::once(word).chain(words.by_ref()).collect::<Vec<_>>().join(" "),
            false => word,
        };
        input.insert(name.clone(), value(name, &[Some(word)], prop)?);
    }
    let rest: Vec<String> = words.collect();
    ensure!(rest.is_empty(), "{} isn't an input: give inputs as --flags (see --help)", rest.join(" "));
    Ok(input)
}

fn required(schema: &Value) -> Vec<String> {
    schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

fn missing(schema: &Value, input: &Map<String, Value>) -> Vec<String> {
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
            Some("integer") => json!(v.parse::<i64>().with_context(|| format!("--{} takes a whole number, not {v}", kebab(flag)))?),
            Some("number") => json!(v.parse::<f64>().with_context(|| format!("--{} takes a number, not {v}", kebab(flag)))?),
            Some("boolean") => match v.as_str() {
                "true" | "yes" | "1" => json!(true),
                "false" | "no" | "0" => json!(false),
                _ => bail!("--{} is true or false, not {v}", kebab(flag)),
            },
            Some("object") => serde_json::from_str(v).with_context(|| format!("--{} takes a JSON object", kebab(flag)))?,
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

/// Fills missing inputs from stdin, when something is piped: a JSON object of inputs, or the one missing input's text.
fn from_stdin(schema: &Value, missing: &[String], input: &mut Map<String, Value>) -> Result<()> {
    if std::io::stdin().is_terminal() {
        return Ok(());
    }
    let Some(text) = piped() else { return Ok(()) };
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
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

/// A local file as an input: `{ name, type, size, base64 }`.
pub fn read_file(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(bytes.len() <= FILE_LIMIT, "{} is over {} MiB", path.display(), FILE_LIMIT >> 20);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok(json!({ "name": name, "type": mime(&name), "size": bytes.len(), "base64": data_encoding::BASE64.encode(&bytes) }))
}

/// A file's type from its name.
pub fn mime(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "zip" => "application/zip",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// A file in a result: an object with `base64` and a `name` or `type` (`mimeType`).
pub fn as_file(v: &Value) -> Option<(String, String, Vec<u8>)> {
    let data = v.get("base64").or_else(|| v.get("data").filter(|_| v.get("mimeType").is_some()))?.as_str()?;
    let ty = v.get("mimeType").or_else(|| v.get("type")).and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let name = v.get("name").and_then(Value::as_str).map(str::to_string);
    if name.is_none() && v.get("type").or_else(|| v.get("mimeType")).is_none() {
        return None;
    }
    let bytes = data_encoding::BASE64.decode(data.as_bytes()).ok()?;
    Some((name.unwrap_or_default(), ty, bytes))
}

/// Prints an operation's result: text as it is, files saved (to `out`, or a temporary file named for them) and their
/// paths printed, anything else as JSON. With `out`, a result that isn't a file is saved there instead.
fn show(app: &str, op: &str, ok: &Value, out: Option<&str>) -> Result<()> {
    let mut out = out.map(std::path::PathBuf::from);
    let mut save = |i: usize, name: &str, ty: &str, bytes: &[u8]| -> Result<()> {
        let path = match out.take() {
            Some(p) => p,
            None => {
                let ext = ty.split('/').nth(1).unwrap_or("bin").split(['+', ';']).next().unwrap_or("bin");
                let name = if name.is_empty() { format!("{app}-{op}-{i}.{ext}") } else { name.replace('/', "_") };
                std::env::temp_dir().join(name)
            }
        };
        std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
        println!("{}", path.display());
        Ok(())
    };
    if let Some(content) = ok["content"].as_array() {
        for (i, c) in content.iter().enumerate() {
            match (c["type"].as_str(), as_file(c)) {
                (Some("image" | "audio"), Some((name, ty, bytes))) => save(i, &name, &ty, &bytes)?,
                (Some("resource"), _) if c["resource"]["blob"].is_string() => {
                    let r = &c["resource"];
                    let bytes = data_encoding::BASE64.decode(r["blob"].as_str().unwrap_or("").as_bytes())?;
                    let name = r["uri"].as_str().and_then(|u| u.rsplit('/').next()).unwrap_or("");
                    save(i, name, r["mimeType"].as_str().unwrap_or(""), &bytes)?;
                }
                _ => println!("{}", c["text"].as_str().or(c["resource"]["text"].as_str()).unwrap_or("")),
            }
        }
        return Ok(());
    }
    if let Some((name, ty, bytes)) = as_file(ok) {
        return save(0, &name, &ty, &bytes);
    }
    let text = match ok {
        Value::String(s) => s.clone(),
        Value::Null => return Ok(()),
        other => serde_json::to_string_pretty(other)?,
    };
    match out {
        Some(path) => {
            std::fs::write(&path, format!("{text}\n")).with_context(|| format!("write {}", path.display()))?;
            println!("{}", path.display());
        }
        None => println!("{text}"),
    }
    Ok(())
}

/// One input's type as help shows it.
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

/// An operation's help, from its description and input schema.
pub fn help(app: &Value, op: &Value) -> String {
    let (a, o) = (app["name"].as_str().unwrap_or(""), op["name"].as_str().unwrap_or(""));
    let schema = &op["input"];
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    let required = required(schema);
    let mut usage = format!("{CLI} {a} {o}");
    for r in &required {
        usage.push_str(&format!(" <{}>", kebab(r)));
    }
    if props.keys().any(|p| !required.contains(p)) {
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
    if app["runs"] == "here" {
        rows.push(("--server <name>".into(), "which of your servers runs it, by name; all for every one; default is this one".into()));
    } else {
        rows.push(("--server <name>".into(), "run it on that server".into()));
    }
    rows.push(("--out <path>".into(), "save what it gives back there".into()));
    if op["confirm"] == true {
        rows.push(("--confirm".into(), "go ahead without asking: it can't be undone".into()));
    }
    let width = rows.iter().map(|(f, _)| f.chars().count()).max().unwrap_or(0).min(32);
    let mut text = format!("Usage: {usage}\n\n{}\n\nInputs:\n", op["description"].as_str().unwrap_or(""));
    for (flag, about) in rows {
        let pad = width.saturating_sub(flag.chars().count());
        text.push_str(format!("  {flag}{}  {about}", " ".repeat(pad)).trim_end());
        text.push('\n');
    }
    text.push('\n');
    text.push_str(match (op["reads"] == true, op["confirm"] == true) {
        (true, _) => "It only reads.\n",
        (_, true) => "It can't be undone: it asks first, or give --confirm.\n",
        _ => "",
    });
    if !required.is_empty() {
        text.push_str(&format!("Inputs not given are read as JSON from stdin: {CLI} {a} {o} < input.json\n"));
    }
    text
}

/// A description's first sentence, for a list.
fn first_sentence(text: &str) -> &str {
    let line = text.lines().next().unwrap_or("");
    match line.find(". ") {
        Some(i) => &line[..=i],
        None => line,
    }
}

/// Every operation of `apps` as the command to type and what it does, aligned.
pub fn list(apps: &[Value]) -> Vec<String> {
    let rows: Vec<(String, String)> = apps
        .iter()
        .flat_map(|a| {
            let name = a["name"].as_str().unwrap_or("").to_string();
            a["ops"].as_array().cloned().unwrap_or_default().into_iter().filter(|o| !o["name"].as_str().unwrap_or("").starts_with('_')).map(move |o| {
                let mark = if o["confirm"] == true { " (can't be undone)" } else { "" };
                (format!("{CLI} {name} {}", o["name"].as_str().unwrap_or("")), format!("{}{mark}", first_sentence(o["description"].as_str().unwrap_or(""))))
            })
        })
        .collect();
    let width = rows.iter().map(|(c, _)| c.chars().count()).max().unwrap_or(0).clamp(28, 36);
    rows.into_iter().map(|(c, d)| format!("  {c:<width$}  {d}")).collect()
}

/// Whether an input schema's property takes a file (`format: "file"`), or a list of them.
fn takes_file(prop: &Value) -> Option<bool> {
    match (prop["format"] == "file", prop["items"]["format"] == "file") {
        (true, _) => Some(false),
        (_, true) => Some(true),
        _ => None,
    }
}

/// How MCP describes a file input to agents.
pub const MCP_FILE: &str = "a file: { base64, name, type } (up to 1 MiB), { file } (a file id another tool gave back) or { url } (public)";

/// An operation's input schema as an MCP tool's: files explained, and `confirm` on what can't be undone.
pub fn mcp_schema(app: &Value, op: &Value) -> Value {
    let mut schema = op["input"].clone();
    if !schema.is_object() {
        schema = json!({ "type": "object", "properties": {} });
    }
    if !schema["properties"].is_object() {
        schema["properties"] = json!({});
    }
    for prop in schema["properties"].as_object_mut().into_iter().flat_map(|p| p.values_mut()) {
        if takes_file(prop).is_some() {
            let about = prop["description"].as_str().map(|d| format!("{d}; ")).unwrap_or_default();
            prop["description"] = json!(format!("{about}{MCP_FILE}"));
        }
    }
    if app["runs"] == "here" {
        schema["properties"]["server"] = json!({ "type": "string", "description": "which of your servers, by name, or all for every one; default is this one" });
    }
    if op["confirm"] == true {
        schema["properties"]["confirm"] = json!({ "type": "boolean", "description": "true once the person has agreed: this can't be undone" });
    }
    schema
}

/// Checks `v` against an input schema, as code mode's interpreter doesn't: types, choices, required inputs, and no
/// inputs an operation doesn't take. `at` names it in what's wrong.
pub fn check(schema: &Value, v: &Value, at: &str) -> Result<()> {
    if let Some(choices) = schema["enum"].as_array() {
        ensure!(choices.contains(v), "{at} is one of {}, not {v}", choices.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", "));
    }
    let types: Vec<&str> = match &schema["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    let is = |t: &str| match t {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    };
    if !types.is_empty() && !types.iter().any(|t| is(t)) {
        let want = types.iter().map(|t| match *t { "string" => "text", "integer" => "a whole number", "number" => "a number", "boolean" => "true or false", "array" => "a list", "object" => "an object", t => t }).collect::<Vec<_>>().join(" or ");
        bail!("{at} takes {want}, not {v}");
    }
    if let (Some(items), Some(list)) = (schema.get("items"), v.as_array()) {
        for (i, x) in list.iter().enumerate() {
            check(items, x, &format!("{at}[{i}]"))?;
        }
    }
    if let (Some(props), Some(o)) = (schema["properties"].as_object(), v.as_object()) {
        for r in required(schema) {
            ensure!(o.get(&r).is_some_and(|x| !x.is_null()), "{at} needs {r}");
        }
        for (k, x) in o {
            match props.get(k) {
                Some(_) if x.is_null() => {}
                Some(p) => check(p, x, &format!("{at}.{k}"))?,
                // An operation's own inputs are all listed; deeper objects may hold more, unless they say not.
                None if (at == "input" && !props.is_empty()) || schema["additionalProperties"] == false => {
                    bail!("{at} has no {k}: it takes {}", props.keys().cloned().collect::<Vec<_>>().join(", "))
                }
                None => {}
            }
        }
    }
    Ok(())
}

/// An MCP tool's hints: whether it only reads, and whether it can't be undone.
pub fn mcp_annotations(op: &Value) -> Value {
    let mut a = json!({});
    if op["reads"] == true {
        a["readOnlyHint"] = json!(true);
    }
    if op["confirm"] == true {
        a["destructiveHint"] = json!(true);
    }
    a
}

/// Brings an agent's file inputs to what apps take, `{ name, type, size, base64 }`: from base64, a file id (a blob
/// this server can get) or a public URL.
pub fn mcp_files(op: &Value, input: &mut Map<String, Value>) -> Result<()> {
    let props = op["input"]["properties"].as_object().cloned().unwrap_or_default();
    for (name, prop) in props {
        let (Some(list), Some(v)) = (takes_file(&prop), input.get_mut(&name)) else { continue };
        match (list, v) {
            (true, Value::Array(items)) => {
                for item in items {
                    *item = mcp_file(item).with_context(|| format!("{name}"))?;
                }
            }
            (_, v) => *v = mcp_file(v).with_context(|| format!("{name}"))?,
        }
    }
    Ok(())
}

fn mcp_file(v: &Value) -> Result<Value> {
    let name = v["name"].as_str().map(str::to_string);
    let (bytes, ty, name) = if let Some(data) = v["base64"].as_str() {
        let bytes = data_encoding::BASE64.decode(data.trim().as_bytes()).context("base64 that doesn't decode")?;
        ensure!(bytes.len() <= MCP_FILE_LIMIT, "a file given as base64 is at most 1 MiB; give a file id or URL for a bigger one");
        (bytes, v["type"].as_str().or(v["mimeType"].as_str()).map(str::to_string), name)
    } else if let Some(id) = v["file"].as_str().or(v["blob"].as_str()) {
        let mut resp = crate::agent().get(format!("http://{}/v1/blobs/{id}", crate::addr())).call().with_context(|| format!("{} isn't running here", one::config::NAME))?;
        ensure!(resp.status().is_success(), "no file {id} for you");
        (resp.body_mut().with_config().limit(FILE_LIMIT as u64).read_to_vec()?, v["type"].as_str().map(str::to_string), name)
    } else if let Some(url) = v["url"].as_str() {
        ensure!(url.starts_with("https://") || url.starts_with("http://"), "a file's url is http or https");
        let mut resp = crate::agent().get(url).call().with_context(|| format!("couldn't get {url}"))?;
        ensure!(resp.status().is_success(), "{url} answered {}", resp.status());
        let ty = resp.headers().get("content-type").and_then(|t| t.to_str().ok()).map(|t| t.split(';').next().unwrap_or(t).to_string());
        let named = url.split(['?', '#']).next().and_then(|u| u.rsplit('/').next()).filter(|n| !n.is_empty()).map(str::to_string);
        (resp.body_mut().with_config().limit(FILE_LIMIT as u64).read_to_vec().with_context(|| format!("{url} is over {} MiB", FILE_LIMIT >> 20))?, v["type"].as_str().map(str::to_string).or(ty), name.or(named))
    } else {
        bail!("give a file as {MCP_FILE}");
    };
    let name = name.unwrap_or_default();
    let ty = ty.unwrap_or_else(|| mime(&name).to_string());
    Ok(json!({ "name": name, "type": ty, "size": bytes.len(), "base64": data_encoding::BASE64.encode(&bytes) }))
}

/// A result as MCP content: text, or JSON with each file in it kept as a file this server serves, given back as a
/// reference (name, type, size, its id and a link), with images also shown.
pub fn mcp_content(ok: &Value) -> Value {
    if ok["content"].is_array() {
        return ok["content"].clone();
    }
    let mut shown = Vec::new();
    let mut ok = ok.clone();
    keep_files(&mut ok, &mut shown, 0);
    let text = match &ok {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    let mut content = vec![json!({ "type": "text", "text": text })];
    content.extend(shown);
    json!(content)
}

/// Replaces each file in `v` (to a few levels deep) with a reference to a blob that holds it.
fn keep_files(v: &mut Value, shown: &mut Vec<Value>, depth: usize) {
    if let Some((name, ty, bytes)) = as_file(v) {
        let put = crate::request(crate::agent().post(format!("http://{}/v1/blobs", crate::addr())).send(&bytes[..]));
        if let Ok(r) = put {
            let id = r["blob"].as_str().unwrap_or("").to_string();
            if ty.starts_with("image/") && bytes.len() <= MCP_FILE_LIMIT {
                shown.push(json!({ "type": "image", "data": data_encoding::BASE64.encode(&bytes), "mimeType": ty }));
            }
            *v = json!({ "name": name, "type": ty, "size": bytes.len(), "file": id, "link": format!("http://{}/v1/blobs/{id}", crate::addr()) });
        }
        return;
    }
    if depth >= 4 {
        return;
    }
    match v {
        Value::Array(items) => items.iter_mut().for_each(|i| keep_files(i, shown, depth + 1)),
        Value::Object(map) => map.values_mut().for_each(|i| keep_files(i, shown, depth + 1)),
        _ => {}
    }
}
