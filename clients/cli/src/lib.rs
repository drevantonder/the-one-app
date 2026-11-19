//! The CLI, a client on One Core in a client (`core/client`) and the native code of the apps it ships with. Every
//! tool is a command: the words before the first `--` input, joined by `_`, are the tool's name, so
//! `one notes add --text milk` calls `notes_add`. `one notes` shows the app's skill and its tools, and
//! `one mcp` serves MCP over stdio instead, or with `--code`, in code mode. `one search <words>` finds the tools for
//! a task, and `one run <file>` runs a script that calls them, as code mode's `execute`. What isn't a tool is in `COMMANDS`.

mod install;
mod mcp;
pub mod serving;
mod system;
mod tasks;

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, fs, io};

use anyhow::{Context, Result, bail};
use client::{Client, Connection, InputResolver, NotApproved, State};
use data_encoding::BASE64;
use serde_json::{Map, Value, json};

/// What the native code of the apps this program ships with fills in of a tool's inputs, which every client it
/// opens hands each call (`client::InputResolver`).
pub fn resolvers() -> Vec<Arc<dyn InputResolver>> {
    vec![Arc::new(encryption_client::Encrypts)]
}

pub fn main() -> ExitCode {
    // A server's environment, where the system gives a program none of its own, as Task Scheduler gives a task's,
    // is given to `one serve run` as its arguments, each NAME=value (`serving`). It's set before anything runs.
    if let [serve, run, args @ ..] = &env::args().skip(1).collect::<Vec<_>>()[..]
        && (serve.as_str(), run.as_str()) == ("serve", "run")
    {
        for (name, value) in args.iter().filter_map(|args| args.split_once('=')) {
            unsafe { env::set_var(name, value) };
        }
    } else {
        // What it prints, piped into a program that stops reading (`one builds | head`), ends it quietly, as it ends
        // `cat` and cargo: the system's own way, which Rust turns off, so printing panics. `one serve run` keeps it
        // off, since it writes to the server it keeps running, which may have gone.
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL)
        };
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio starts");
    let args: Vec<String> = env::args().skip(1).collect();
    // Run by git as its remote helper, it's that, under the name git found it by, with `.exe` after it on Windows.
    let named = env::args().next().map(|program| Path::new(&program).file_stem().map(|name| name.to_owned()));
    let result = if named.flatten().is_some_and(|name| name == git_repos_client::HELPER) {
        runtime.block_on(async { git_repos_client::helper(Client::open(client::home()?, resolvers(), Vec::new()).await?, &args).await })
    } else {
        git_repos_client::beside();
        runtime.block_on(run(args))
    };
    result.unwrap_or_else(|error| {
        eprintln!("{error:#}");
        ExitCode::FAILURE
    })
}

/// What isn't a tool: finding tools and running scripts with them, MCP, and what's about this device: its build,
/// making it a server (`serving`), and updating it.
const COMMANDS: &str = "Besides the tools:\n\n\
                           one serve on       makes it a server of yours, which starts whenever it does\n\
                           one serve off      stops it being one, and removes it\n\
                           one update         takes the latest build: this device's, or on a server the network's Update Now\n\
                           one search <words> finds the tools for a task, each with its command\n\
                           one run <file>     runs a script that calls tools, as MCP's execute does\n\
                           one task <id>      says how a tool called with --task is going; --result waits for its answer\n\
                           one mcp [--code]   serves MCP to an agent here: every tool, or code mode's few\n\
                           one restore        restores a whole network from a backup, with none running\n\
                           one --version      says which build it is";

async fn run(args: Vec<String>) -> Result<ExitCode> {
    if args == ["--version"] {
        println!("one {} {}", client::BUILD, client::target());
        return Ok(ExitCode::SUCCESS);
    }
    // What the system keeps running for a server's services, which needs nothing of the network (`system`).
    if args.first().is_some_and(|word| word == "system") {
        return Ok(printed(system::run(&args[1..])?));
    }
    // A network is restored where there's none running.
    if args.first().is_some_and(|word| word == "restore") {
        return Ok(printed(backups_client::restore(&args[1..]).await?));
    }
    let client = Client::open(client::home()?, resolvers(), Vec::new()).await?;
    signed_in(&client).await?;
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = match words.as_slice() {
        ["mcp"] | ["mcp", "--code"] => {
            mcp::serve(client.clone(), words.len() == 2).await?;
            ExitCode::SUCCESS
        }
        ["run", file] => {
            let code = fs::read_to_string(file).with_context(|| format!("couldn't read {file}"))?;
            let request = json!({"name": "execute", "arguments": {"code": code}, "_meta": {"mode": "code"}});
            let result = client.request("tools/call", request).await?;
            println!("{}", result["structuredContent"]["text"].as_str().unwrap_or_default());
            if result["isError"] == true { ExitCode::FAILURE } else { ExitCode::SUCCESS }
        }
        ["search", query @ ..] if !query.is_empty() && !query[0].starts_with("--") => {
            let request = json!({"name": "search", "arguments": {"query": query.join(" ")}, "_meta": {"mode": "cli"}});
            let result = client.request("tools/call", request).await?;
            printed(text(&result))
        }
        ["serve", "on"] => printed(serving::on(&client).await?),
        ["serve", "off"] => printed(serving::off(&client).await?),
        ["serve", "run", ..] => {
            serving::run(client.clone()).await?;
            ExitCode::SUCCESS
        }
        ["update"] => printed(serving::update(&client).await?),
        ["task", id] => tasks::show(&client, id, false).await?,
        ["task", id, "--result"] => tasks::show(&client, id, true).await?,
        // An app from a git repo, as `npx skills add` adds a skill; with no repo, the app is the network's own.
        ["install", repo, ..] if !repo.starts_with("--") => install::install(&client, &args[1..]).await?,
        ["install"] if std::path::Path::new("apps.lock").exists() => install::locked(&client).await?,
        ["uninstall", "--name", name] if std::path::Path::new("apps.lock").exists() => {
            install::uninstall(&client, name).await?
        }
        ["deploy", ..] if words.contains(&"--files-from") => install::deploy_folder(&client, &args).await?,
        _ => command(&client, &args).await?,
    };
    client.close().await;
    Ok(code)
}

/// Waits until this device is in its network. One that isn't says its code and where it's let in, and carries on
/// once it is; one whose code was denied or ended, or that's opening while its network doesn't answer, says so and
/// stops.
async fn signed_in(client: &Client) -> Result<()> {
    let mut state = client.state();
    loop {
        match &*state.borrow_and_update() {
            State::SignedIn { .. } => return Ok(()),
            State::Opening { connection: Connection::Offline } => bail!("{}", Connection::Offline),
            State::Opening { .. } => {}
            out @ State::SignIn { why: Some(NotApproved::Denied | NotApproved::Expired), .. } => bail!("{out}"),
            out @ State::SignIn { .. } => eprintln!("{out}"),
        }
        state.changed().await?;
    }
}

fn printed(text: String) -> ExitCode {
    println!("{text}");
    ExitCode::SUCCESS
}

/// What a tool answered, `result`, as text: its parts' text, a line each.
pub(crate) fn text(result: &Value) -> String {
    let parts = result["content"].as_array().into_iter().flatten();
    parts.filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n")
}

/// What a tool answered, `result`, as text, or the error it says.
pub(crate) fn said(result: Value) -> Result<String> {
    let text = text(&result);
    if result["isError"] == true {
        bail!("{text}")
    }
    Ok(text)
}

/// Prints what a tool answered: its text, or a file as it is, to be sent to a file (`> photo.jpg`). An answer that's
/// an error is printed to stderr, and fails.
pub(crate) fn answered(client: &Client, result: &Value) -> Result<ExitCode> {
    let parts = result["content"].as_array().map(Vec::as_slice).unwrap_or_default();
    if result["isError"] == true {
        eprintln!("{}", text(result));
        return Ok(ExitCode::FAILURE);
    }
    for part in parts {
        match part["text"].as_str() {
            Some(text) => println!("{text}"),
            None => {
                // A large file is one the client fetched and kept, and any other is in the answer.
                let file = match part["uri"].as_str().and_then(|uri| uri.strip_prefix("/.file/")) {
                    Some(path) => client.file(path).context("the answer isn't a file")?.1,
                    None => {
                        let file = part["data"].as_str().or(part["resource"]["blob"].as_str());
                        BASE64.decode(file.unwrap_or_default().as_bytes()).context("the answer isn't a file")?
                    }
                };
                io::stdout().write_all(&file)?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Calls the tool `args` name, or shows what they could have named.
pub(crate) async fn command(client: &Client, args: &[String]) -> Result<ExitCode> {
    let (words, flags) = args.split_at(args.iter().take_while(|arg| !arg.starts_with("--")).count());
    let name = words.join("_");
    let (listed, listed_result) = match client.fresh("tools/list", &json!({})).await {
        Ok(listed) => (listed, Ok(())),
        Err(error) => (client.cached("tools/list", &json!({})).unwrap_or_default(), Err(error)),
    };
    let tools = listed["tools"].as_array().map(Vec::as_slice).unwrap_or_default();

    match tools.iter().find(|tool| tool["name"] == name.as_str()) {
        Some(tool) if flags.iter().any(|flag| flag == "--help") => usage(tool),
        Some(tool) => {
            listed_result?;
            let task = flags.iter().any(|flag| flag == "--task");
            let flags: Vec<String> = flags.iter().filter(|flag| *flag != "--task").cloned().collect();
            let inputs = inputs(client, tool, &flags).await?;
            return tasks::call(client, &name, inputs, task).await;
        }
        None => {
            let within: Vec<&Value> = tools
                .iter()
                .filter(|tool| {
                    name.is_empty() || tool["name"].as_str().is_some_and(|tool| tool.starts_with(&format!("{name}_")))
                })
                .collect();
            // What a server says now, or what one last said if none is answering.
            let says = async |method: &str, params: Value| match listed_result {
                Ok(()) => client.fresh(method, &params).await.ok(),
                Err(_) => client.cached(method, &params),
            };
            // An app with no tools can still have a skill: the UI library's says how an app is written with it. It's
            // read as any skill is, and what's printed of it is what follows its name and description. An app's
            // own skill is named here by the app's name, as it's added, and One Core's by its name. Where both
            // are, the app's is shown with its tools.
            let skills = says("skills/list", json!({})).await.unwrap_or_default();
            let skills = skills["skills"].as_array().map(Vec::as_slice).unwrap_or_default();
            let uri = skills.iter().filter_map(|skill| skill["uri"].as_str()).rfind(|uri| uri_words(uri) == words);
            let skill = match uri {
                Some(uri) => says("resources/read", json!({"uri": uri})).await,
                None => None,
            };
            let skill = skill.as_ref().and_then(|skill| skill["contents"][0]["text"].as_str());
            let skill = skill.map(|skill| skill.split_once("\n---\n").map_or(skill, |(_, text)| text).trim());
            if within.is_empty() && skill.is_none() {
                listed_result?;
                bail!("There's no tool, app or skill named `{}`. `one --help` lists them.", words.join(" "));
            }
            if listed_result.is_err() {
                eprintln!("No server is answering, so this is what one last listed.\n");
            }
            if let Some(skill) = skill {
                println!("{skill}\n");
            }
            if within.is_empty() {
                return Ok(ExitCode::SUCCESS);
            }
            let width = within.iter().filter_map(|tool| tool["name"].as_str()).map(str::len).max().unwrap_or(0);
            for tool in within {
                let name = tool["name"].as_str().unwrap_or_default().replacen('_', " ", 1);
                let about = tool["description"].as_str().unwrap_or_default().lines().next().unwrap_or_default();
                println!("one {name:width$}  {about}");
            }
            println!("\n`one <tool> --help` shows a tool's inputs.");
            if name.is_empty() {
                println!("\n{COMMANDS}");
                println!("\nA skill says when and how to use an app's tools, and `one <skill>` shows one:\n");
                for skill in skills {
                    let words = uri_words(skill["uri"].as_str().unwrap_or_default()).join(" ");
                    let about = skill["frontmatter"]["description"].as_str().unwrap_or_default();
                    println!("one {words:width$}  {about}");
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The words that show a skill, from its address (`OneCore.Skills`): an app's own, `skill://<app>/<its own
/// name>/SKILL.md`, by the app's name; one of its `skills/` by the app's name and the skill's; and One Core's
/// own, `skill://<name>/SKILL.md`, by its name.
fn uri_words(uri: &str) -> Vec<&str> {
    let path = uri.strip_prefix("skill://").and_then(|uri| uri.strip_suffix("/SKILL.md")).unwrap_or_default();
    match path.split('/').collect::<Vec<_>>().as_slice() {
        [app, _own] => vec![app],
        [app, .., name] => vec![app, name],
        parts => parts.to_vec(),
    }
}

/// A tool's inputs from the command line: `--name value`, typed as the tool says (`value`), or `--name-from path`
/// to read the value from a file. A list takes its items one after another, or again with each `--name`.
/// `--no-name` is nothing (`null`) for an input that takes it, and `false` for one that's true or false, which
/// `--name` alone is `true` for. A file for an input that says it's an upload (its schema's `format`) is sent to
/// where the network keeps it, and the input is the name it was given there.
async fn inputs(client: &Client, tool: &Value, flags: &[String]) -> Result<Value> {
    let known = &tool["inputSchema"]["properties"];
    let mut inputs = Map::new();
    let mut flags = flags.iter().peekable();
    while let Some(flag) = flags.next() {
        let name = flag.strip_prefix("--").with_context(|| format!("`{flag}` isn't an input: write --name value"))?;
        if let Some(none) =
            name.strip_prefix("no-").filter(|none| known.get(name).is_none() && known.get(none).is_some())
        {
            let value = match kinds(&known[none]) {
                kinds if kinds.contains(&"null") => Value::Null,
                kinds if kinds.contains(&"boolean") => Value::Bool(false),
                _ => bail!("--{none} can't be nothing. Leave it out, or give it {}.", takes(&known[none])),
            };
            inputs.insert(none.to_owned(), value);
            continue;
        }
        let (name, from_file) = match name.strip_suffix("-from") {
            Some(name) if known.get(name).is_some() => (name, true),
            _ => (name, false),
        };
        let schema = known.get(name).with_context(|| format!("It has no input named `{name}`. --help shows them."))?;
        let kinds = kinds(schema);
        let value = match flags.next_if(|next| !next.starts_with("--")) {
            None if kinds.contains(&"boolean") => Value::Bool(true),
            None => bail!("--{name} needs {}", takes(schema)),
            Some(path) if from_file && schema["format"] == "upload" => {
                let file = fs::read(path).with_context(|| format!("couldn't read {path}"))?;
                let app = tool["name"].as_str().and_then(|tool| tool.split('_').next()).unwrap_or_default();
                Value::String(client.upload(app, file).await?)
            }
            Some(path) if from_file => read(Path::new(path), schema["contentEncoding"] == "base64")?,
            // A list's items are the words up to the next input, unless it's written as JSON.
            Some(word) if kinds.contains(&"array") && !word.starts_with('[') => {
                let mut items = vec![value(name, &schema["items"], word)?];
                while let Some(word) = flags.next_if(|next| !next.starts_with("--")) {
                    items.push(value(name, &schema["items"], word)?);
                }
                match inputs.remove(name) {
                    Some(Value::Array(before)) => Value::Array([before, items].concat()),
                    _ => Value::Array(items),
                }
            }
            Some(word) => value(name, schema, word)?,
        };
        inputs.insert(name.to_owned(), value);
    }
    Ok(Value::Object(inputs))
}

/// The kinds of value a schema takes, by JSON Schema's names: its `type`, one or several. None for a schema that
/// doesn't say, which takes any JSON.
fn kinds(schema: &Value) -> Vec<&str> {
    match &schema["type"] {
        Value::String(kind) => vec![kind.as_str()],
        Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    }
}

/// One word as a value `schema` takes: a word that's JSON of a kind it takes besides text and nothing is that
/// (`12`, `true`, `["a"]`), and any other word is text, where it takes text. Nothing is `--no-<input>`, so a word
/// is never it, and `null` is the text it says.
fn value(name: &str, schema: &Value, word: &str) -> Result<Value> {
    let kinds = kinds(schema);
    let is = |value: &Value, kind: &str| match kind {
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    };
    let json = serde_json::from_str::<Value>(word).ok();
    match json.filter(|value| kinds.is_empty() && !value.is_null() || kinds.iter().any(|kind| is(value, kind))) {
        Some(value) => Ok(value),
        None if kinds.is_empty() || kinds.contains(&"string") => Ok(Value::String(word.to_owned())),
        None => bail!("--{name} takes {}, which `{word}` isn't", takes(schema)),
    }
}

/// What a schema takes, as `--help` says it: one of a few as the few, `text`, `a number` and the rest.
fn takes(schema: &Value) -> String {
    let one_of: Vec<&str> = schema["enum"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !one_of.is_empty() {
        return one_of.join("|");
    }
    let taken: Vec<String> = kinds(schema)
        .into_iter()
        .filter(|kind| *kind != "null")
        .map(|kind| match kind {
            "string" => "text".to_owned(),
            "integer" => "a whole number".to_owned(),
            "number" => "a number".to_owned(),
            "boolean" => "true|false".to_owned(),
            "array" => format!("{} ...", takes(&schema["items"])),
            _ => "JSON".to_owned(),
        })
        .collect();
    if taken.is_empty() { "JSON".to_owned() } else { taken.join(" or ") }
}

/// A file's text, or a folder as its files by their paths in it, as `client::files` reads a folder: each one's text,
/// or `{"base64": ...}` for one that isn't text, as `deploy` takes an app's files. A file for an input that says
/// it's base64 (its schema's `contentEncoding`) is read as it is and written as that.
pub(crate) fn read(path: &Path, base64: bool) -> Result<Value> {
    let couldnt = || format!("couldn't read {}", path.display());
    if base64 {
        return Ok(Value::String(BASE64.encode(&fs::read(path).with_context(couldnt)?)));
    }
    if !path.is_dir() {
        return Ok(Value::String(fs::read_to_string(path).with_context(couldnt)?));
    }
    let mut into = Map::new();
    for file in client::files(path).with_context(couldnt)? {
        let data = fs::read(path.join(&file)).with_context(couldnt)?;
        let value = match String::from_utf8(data) {
            Ok(text) => Value::String(text),
            Err(data) => json!({"base64": BASE64.encode(data.as_bytes())}),
        };
        into.insert(file, value);
    }
    Ok(Value::Object(into))
}

fn usage(tool: &Value) {
    println!("one {}\n", tool["name"].as_str().unwrap_or_default().replacen('_', " ", 1));
    println!("{}\n", tool["description"].as_str().unwrap_or_default().trim_end());
    let required = tool["inputSchema"]["required"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut inputs: Vec<_> = tool["inputSchema"]["properties"].as_object().into_iter().flatten().collect();
    inputs.sort_by_key(|(name, _)| !required.iter().any(|required| required == *name));
    for (name, input) in inputs {
        let optional = if required.iter().any(|required| required == name) { "" } else { ", optional" };
        let about = input["description"].as_str().unwrap_or_default();
        let none = if kinds(input).contains(&"null") { format!(" or --no-{name}") } else { String::new() };
        println!("  --{name} <{}{optional}>{none}  {about}", takes(input));
    }
    println!("\nA list takes its items one after another. --no-<input> is nothing, where an input says it takes that.");
    println!("--<input>-from <path> reads an input from a file, or from a folder's files that git doesn't ignore.");
}
