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
    let mut apps = get("/v1/apps").ok()?.as_array()?.clone();
    apps.extend(node_apps());
    let a = apps.iter().find(|a| a["name"] == app)?.clone();
    let o = a["ops"].as_array()?.iter().find(|o| o["name"] == op)?.clone();
    Some((a, o))
}

/// The node's own tools, as apps: what search lists, which run here rather than as an app's call; and widgets_show,
/// which MCP has too (src/mcp_apps.rs), as text here.
fn node_apps() -> [Value; 4] {
    let widgets = crate::mcp_apps::tools().into_iter().filter(|t| t["name"] == "widgets_show").collect();
    [one::skills::vault_app(), one::skills::backups_app(), one::skills::groups_app(), one::skills::node_app("widgets", "Widgets", "Your widgets, as text", "", widgets)]
}

/// Whether an app is one of the node's own tools, which run here, as the person: never on another device.
pub fn node_app(app: &str) -> bool {
    ["vault", "backups", "groups", "widgets"].contains(&app)
}

/// `<CLI> <app> <op> [words] [--input value]...`: runs it and prints what it gives back.
pub fn run(app: &str, op: &str, args: &[String]) -> Result<()> {
    let (mut words, mut flags) = split(args);
    // A last word `-` asks for inputs from stdin even when none is missing; otherwise stdin is read only when one is,
    // so a command inside a `while read` loop doesn't eat the loop's input.
    let asked = words.last().is_some_and(|w| w == "-") && words.pop().is_some();
    let spec = find(app, op);
    let Some((a, o)) = &spec else { bail!("{}", unknown(app, op)) };
    if flags.contains_key("help") || flags.contains_key("h") {
        print!("{}", help(a, o));
        return Ok(());
    }
    let mut take = |k: &str| flags.remove(k).and_then(|v| v.into_iter().last().flatten());
    let device = if own_device(o) || node_app(app) { None } else { take("device") };
    let scopes = take("scopes").map(|s| s.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>());
    let out = take("out");
    let confirmed = flags.remove("confirm").is_some() || flags.remove("yes").is_some();
    let files: Vec<String> = flags.remove("file").unwrap_or_default().into_iter().flatten().collect();
    let schema = o["input"].clone();
    let places = places(o);
    let mut input = input(&schema, &places, words, flags, &files)?;
    let missing = missing(&schema, &input);
    if !missing.is_empty() || asked {
        from_stdin(&schema, &missing, &mut input)?;
        let missing = self::missing(&schema, &input);
        ensure!(
            missing.is_empty(),
            "missing {}: give {} as flags, or pipe the inputs as JSON: {CLI} {app} {op} < input.json. See {CLI} {app} {op} --help.",
            missing.join(", "),
            if missing.len() == 1 { "it" } else { "them" }
        );
    }
    if o["confirm"] == true {
        agreed(confirmed, &format!("{CLI} {app} {op}"))?;
    }
    let call = match node_app(app) {
        true => crate::dispatch(&format!("{app}_{op}"), input, agent_from_env().as_deref(), "cli", Some(o.clone())).map(|ok| json!({ "ok": ok })),
        false => post("/v1/call", json!({ "device": device, "scopes": scopes, "app": app, "op": op, "input": input, "agent": agent_from_env(), "client": "cli" })),
    };
    let mut r = match call {
        Ok(r) => r,
        Err(e) => bail!("{}", in_cli_words(&format!("{e:#}"))),
    };
    if let Some(none) = none_said(a, op, &r["ok"], true) {
        println!("{none}");
        return Ok(());
    }
    if let Some(text) = r["ok"].as_str() {
        r["ok"] = json!(in_cli_words(text));
    }
    show(app, op, &r["ok"], out.as_deref())
}

/// What an app says, as the command line says it: each tool it names (`gmail_sign_in`) as its command
/// (`one gmail sign_in`).
fn in_cli_words(text: &str) -> String {
    let mut apps = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
    apps.extend(node_apps());
    let command = |word: &str| -> Option<String> {
        word.match_indices('_').find_map(|(i, _)| {
            let (app, op) = (&word[..i], &word[i + 1..]);
            let a = apps.iter().find(|a| a["name"] == app)?;
            a["ops"].as_array()?.iter().any(|o| o["name"] == op).then(|| format!("{CLI} {app} {op}"))
        })
    };
    let mut out = String::new();
    let mut word = String::new();
    for c in text.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            word.push(c);
            continue;
        }
        out.push_str(&command(&word).unwrap_or_else(|| word.clone()));
        word.clear();
        if c != '\0' {
            out.push(c);
        }
    }
    out
}

/// An operation as the caller names it: `one gmail sign_in` on the command line, `gmail_sign_in` to MCP.
pub fn named(app: &str, op: &str, cli: bool) -> String {
    match cli {
        true => format!("{CLI} {app} {op}"),
        false => format!("{app}_{op}"),
    }
}

/// An empty list of accounts in words, with what to do: screens want the list, a person or agent reading wants this.
pub fn none_said(app: &Value, op: &str, ok: &Value, cli: bool) -> Option<String> {
    if op != "accounts" || ok.as_array().is_none_or(|a| !a.is_empty()) {
        return None;
    }
    let (name, title) = (app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or(""));
    let signs_in = app["ops"].as_array().into_iter().flatten().any(|o| o["name"] == "sign_in");
    // One that needs a part on a device, with none of yours running it: signing in can't work yet.
    let parts: Vec<&Value> = app["uses"]["sidecars"].as_array().into_iter().flatten().collect();
    let on_none = || {
        let state = get("/v1/state").unwrap_or_default();
        !state["install"]["devices"].as_array().into_iter().flatten().any(|d| d["sidecars"].as_array().into_iter().flatten().any(|s| parts.contains(&s)))
    };
    Some(match (signs_in, app["needs"].as_str()) {
        (_, Some(needs)) if !parts.is_empty() && on_none() => format!("None: {title} isn't on any of your devices yet. {needs}."),
        (true, _) => format!("No account is signed in to {title} yet: {} signs one in.", named(name, "sign_in", cli)),
        (false, Some(needs)) => format!("None: {title} isn't on any of your devices yet. {needs}."),
        (false, None) => format!("None: {title} has no accounts yet."),
    })
}

/// What to say for an app or operation nobody here lists.
fn unknown(app: &str, op: &str) -> String {
    let mut apps = get("/v1/apps").ok().and_then(|a| a.as_array().cloned()).unwrap_or_default();
    apps.extend(node_apps());
    if apps.iter().any(|a| a["name"] == app) {
        return format!("{app} has no operation {op}; {CLI} {app} lists them");
    }
    // A tool's name as a command, one repos_create notes: the command is one repos create notes.
    let tool = app.match_indices('_').find_map(|(i, _)| {
        let (a, o) = (&app[..i], &app[i + 1..]);
        apps.iter().find(|x| x["name"] == a)?["ops"].as_array()?.iter().any(|x| x["name"] == o).then(|| format!("{a} {o}"))
    });
    if let Some(command) = tool {
        return format!("no app {app}: did you mean {CLI} {command} {op}?");
    }
    // One the App Store has that isn't added where the caller sees it.
    let listed = post("/v1/call", json!({ "app": "apps", "op": "list", "input": {}, "client": "cli" })).ok();
    let line = listed.as_ref().and_then(|l| l["ok"].as_str()).and_then(|l| l.lines().find(|l| l.contains(&format!(" ({app}): not added"))).map(String::from));
    match line.as_deref().and_then(|l| l.split_once(&format!(" ({app})"))) {
        Some((title, _)) => format!("{title} isn't added yet: {CLI} apps add {app} user adds it for you, or {CLI} apps add {app} global for everyone"),
        None => format!("no app {app}; {CLI} apps lists them"),
    }
}

/// Goes ahead with `command`, which sends or can't be undone, given --confirm; else asks at a terminal, and elsewhere
/// (an agent, or piped) refuses.
pub fn agreed(confirmed: bool, command: &str) -> Result<()> {
    if confirmed {
        return Ok(());
    }
    ask(&format!("{command} sends or can't be undone. Go ahead?"), &format!("{command} sends or can't be undone, and the person hasn't agreed to it: ask them, and give --confirm only if they say yes to this very action"))
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

/// The inputs words fill, in order: the operation's `words` when it names them, else its required inputs.
fn places(op: &Value) -> Vec<String> {
    match op["words"].as_array() {
        Some(words) => words.iter().filter_map(Value::as_str).map(String::from).collect(),
        None => required(&op["input"]),
    }
}

/// The operation's input from flags, words (filling `places` in order) and files. Given fewer words than places, the
/// optional places are skipped first: `one board post "hi"` is text, `one board post ideas "hi"` a topic and text.
/// A word left over is an error, never folded into another input: it says to quote text, or which flag to give it as.
/// Whether an operation takes a `device` of its own (an agent device's name, say): then that's what `device` is, and
/// it says itself where it runs, so the CLI's and MCP's `device` (which of your devices runs it) isn't offered.
pub fn own_device(op: &Value) -> bool {
    op["input"]["properties"].get("device").is_some()
}

/// Whether an operation's input takes `name`: one of its properties, or anything where it's open-ended. A name it
/// doesn't take (one renamed, say) fails, rather than being passed over and the call run as if it weren't given.
pub fn takes(schema: &Value, name: &str) -> bool {
    schema["properties"].get(name).is_some() || schema["additionalProperties"] == true || !schema["properties"].is_object()
}

fn input(schema: &Value, places: &[String], words: Vec<String>, flags: BTreeMap<String, Vec<Option<String>>>, files: &[String]) -> Result<Map<String, Value>> {
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
            few if !few.is_empty() && few.len() <= 3 => format!("did you mean {}?", few.iter().map(|p| format!("--{} {first}", kebab(p))).collect::<Vec<_>>().join(" or ")),
            _ => "give inputs as --flags".to_string(),
        }
    );
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
        // Laid out for a person at a terminal; compact for an agent's shell or a pipe.
        other if std::io::stdout().is_terminal() => serde_json::to_string_pretty(other)?,
        other => serde_json::to_string(other)?,
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
    let places = places(op);
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
    if !own_device(op) && !node_app(app["name"].as_str().unwrap_or("")) {
        let about = if app["runs"] == "here" { "which of your devices runs it, by name; all for every one; default is this one" } else { "run it on that device (by name)" };
        rows.push(("--device <name>".into(), about.into()));
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
        (_, true) => "It sends or can't be undone: it asks first, or give --confirm once the person agreed.\n",
        _ => "",
    });
    if !props.is_empty() {
        text.push_str(&format!("Required inputs not given are read as JSON from stdin: {CLI} {a} {o} < input.json; a last word - reads stdin anyway: {CLI} {a} {o} - < input.json\n"));
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
    if app["runs"] == "here" && !own_device(op) {
        schema["properties"]["device"] = json!({ "type": "string", "description": "which of your devices, by name, or all for every one; default is this one" });
    }
    if op["confirm"] == true {
        schema["properties"]["confirm"] = one::skills::confirm_input();
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

/// An operation's MCP hints, from what it says: `reads`, and `confirm`.
pub fn mcp_annotations(op: &Value) -> Value {
    one::skills::hints(op["reads"] == true, op["confirm"] == true)
}

/// Brings an agent's file inputs to what apps take, `{ name, type, size, base64 }`: from base64, a file id (a blob
/// this device can get) or a public URL.
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
        let mut resp = crate::to_node(crate::agent().get(format!("http://{}/v1/blobs/{id}", crate::addr()))).call().with_context(|| format!("{} isn't running here", one::config::NAME))?;
        ensure!(resp.status().is_success(), "no file {id} for you");
        (resp.body_mut().with_config().limit(FILE_LIMIT as u64).read_to_vec()?, v["type"].as_str().map(str::to_string), name)
    } else if let Some(url) = v["url"].as_str() {
        ensure!(url.starts_with("https://") || url.starts_with("http://"), "a file's url is http or https");
        let mut resp = fetch_file(url)?;
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

/// Gets a file an agent gave as a URL. For someone signed in elsewhere, only from the internet, each redirect too: this
/// machine's addresses (its local API, which takes this device for its own programs), its network's and its tailnet's
/// are its person's, not theirs.
fn fetch_file(url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    if !crate::signed_in() {
        return crate::agent().get(url).call().with_context(|| format!("couldn't get {url}"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into();
    let mut at = url.to_string();
    for _ in 0..10 {
        let uri: ureq::http::Uri = at.parse().with_context(|| format!("{at} isn't a URL"))?;
        let host = uri.host().with_context(|| format!("{at} has no host"))?.trim_matches(['[', ']']);
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("https") { 443 } else { 80 });
        let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(&(host, port)).with_context(|| format!("couldn't find {host}"))?.collect();
        ensure!(!addrs.is_empty() && addrs.iter().all(|a| on_the_internet(a.ip())), "{at} isn't on the internet: a file's url is public");
        let resp = agent.get(&at).call().with_context(|| format!("couldn't get {at}"))?;
        if !resp.status().is_redirection() {
            return Ok(resp);
        }
        let to = resp.headers().get("location").and_then(|l| l.to_str().ok()).with_context(|| format!("{at} redirects nowhere"))?;
        at = match to.starts_with('/') {
            true => format!("{}://{}{to}", uri.scheme_str().unwrap_or("http"), uri.authority().map(|a| a.as_str()).unwrap_or(host)),
            false => to.to_string(),
        };
        ensure!(at.starts_with("https://") || at.starts_with("http://"), "{url} redirects to {at}, which isn't http or https");
    }
    bail!("{url} redirects too many times")
}

/// Whether an address is on the internet: not this machine's, a private network's, a tailnet's (100.64.0.0/10,
/// fd7a:115c:a1e0::/48) or a link's.
fn on_the_internet(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() || a == 0 || (a == 100 && (64..128).contains(&b)))
        }
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => on_the_internet(v4.into()),
            None => !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || v6.is_unique_local() || v6.is_unicast_link_local()),
        },
    }
}

/// A result as MCP content: text, or JSON with each file in it kept as a file this device serves, given back as a
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
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let mut content = vec![json!({ "type": "text", "text": text })];
    content.extend(shown);
    json!(content)
}

/// Replaces each file in `v` (to a few levels deep) with a reference to a blob that holds it.
fn keep_files(v: &mut Value, shown: &mut Vec<Value>, depth: usize) {
    if let Some((name, ty, bytes)) = as_file(v) {
        let put = crate::request(crate::to_node(crate::agent().post(format!("http://{}/v1/blobs", crate::addr()))).send(&bytes[..]));
        if let Ok(r) = put {
            let id = r["blob"].as_str().unwrap_or("").to_string();
            if ty.starts_with("image/") && bytes.len() <= MCP_FILE_LIMIT {
                shown.push(json!({ "type": "image", "data": data_encoding::BASE64.encode(&bytes), "mimeType": ty }));
            }
            // No link to it: a browser can't prove to the node it's one of this machine's own programs (src/api.rs);
            // `file get` saves it.
            *v = json!({ "name": name, "type": ty, "size": bytes.len(), "file": id });
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Someone signed in elsewhere can't have this device fetch a file from its own addresses, which would reach its
    /// local API as this machine (the Vault, apps' code, sidecars' calls), or from its networks.
    #[test]
    fn a_sign_in_gets_files_only_from_the_internet() {
        crate::SIGNED_IN.with(|t| *t.borrow_mut() = Some("a sign-in's token".into()));
        for url in ["http://127.0.0.1:4747/v1/vault?scope=user", "http://localhost:4747/v1/source/board", "http://[::1]:4747/v1/state", "http://0.0.0.0:4747/v1/state", "http://192.168.0.1/", "http://100.100.100.100/", "http://[::ffff:127.0.0.1]:4747/v1/vault"] {
            let err = mcp_file(&json!({ "url": url })).unwrap_err();
            assert!(format!("{err:#}").contains("isn't on the internet"), "{url}: {err:#}");
        }
        crate::SIGNED_IN.with(|t| *t.borrow_mut() = None);
        assert!(on_the_internet("1.1.1.1".parse().unwrap()) && on_the_internet("2606:4700:4700::1111".parse().unwrap()));
    }
}
