//! The CLI, a client on One Core in a client (`core/client`) and the native code of the plugins it ships with. Every
//! tool is a command: the words before the first `--` input, joined by `_`, are the tool's name, so
//! `one notes add --text milk` calls `notes_add`. `one notes` shows the plugin's skill and its tools, and
//! `one mcp` serves MCP over stdio instead. A few words are about this device instead (`THIS_DEVICE`).

mod install;
mod mcp;
#[cfg(unix)]
pub mod serving;

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, fs, io};

use anyhow::{Context, Result, bail};
use client::{Client, DeviceAuthorization, InputResolver};
use data_encoding::BASE64;
use serde_json::{Map, Value, json};

/// What the native code of the plugins this program ships with fills in of a tool's inputs, which every client it
/// opens hands each call (`client::InputResolver`).
pub fn resolvers() -> Vec<Arc<dyn InputResolver>> {
    vec![Arc::new(encryption_client::Encrypts)]
}

pub fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio starts");
    let args: Vec<String> = env::args().skip(1).collect();
    // Run by git as its remote helper, it's that, under the name git found it by, with `.exe` after it on Windows.
    let named = env::args().next().map(|program| Path::new(&program).file_stem().map(|name| name.to_owned()));
    let answered = if named.flatten().is_some_and(|name| name == repos_client::HELPER) {
        runtime.block_on(async { repos_client::helper(Client::open(client::home()?, resolvers()).await?, &args).await })
    } else {
        repos_client::beside();
        runtime.block_on(run(args))
    };
    answered.unwrap_or_else(|error| {
        eprintln!("{error:#}");
        ExitCode::FAILURE
    })
}

/// What's about this device rather than a tool: its build, making it a server (`serving`), and updating it.
const THIS_DEVICE: &str = "This device:\n\n\
                           one serve on       makes it a server of yours, which starts whenever it does\n\
                           one serve off      stops it being one, and removes it\n\
                           one update         takes the latest builds\n\
                           one restore        restores a whole network from a backup, with none running\n\
                           one --version      says which build it is";

async fn run(args: Vec<String>) -> Result<ExitCode> {
    if args == ["--version"] {
        println!("one {} {}", client::BUILD, client::target());
        return Ok(ExitCode::SUCCESS);
    }
    // A network is restored where there's none running.
    if args.first().is_some_and(|word| word == "restore") {
        return Ok(printed(backups_client::restore(&args[1..]).await?));
    }
    // A device that isn't in its network yet says its code and where it's let in, and carries on once it is.
    let client = match Client::open(client::home()?, resolvers()).await {
        Err(error) if error.is::<DeviceAuthorization>() => {
            let not_in = error.downcast::<DeviceAuthorization>()?;
            eprintln!("{not_in}");
            not_in.answered().await?;
            Client::open(client::home()?, resolvers()).await?
        }
        opened => opened?,
    };
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = match words.as_slice() {
        ["mcp"] => {
            mcp::serve(client.clone()).await?;
            ExitCode::SUCCESS
        }
        #[cfg(unix)]
        ["serve", "on"] => printed(serving::on(&client).await?),
        #[cfg(unix)]
        ["serve", "off"] => printed(serving::off(&client).await?),
        #[cfg(unix)]
        ["serve", "run"] => {
            serving::run(client.clone()).await?;
            ExitCode::SUCCESS
        }
        #[cfg(unix)]
        ["update"] => printed(serving::update(&client).await?),
        // A plugin from a git repo, as `npx skills add` adds a skill; with no repo, the plugin is the network's own.
        ["install", repo, ..] if !repo.starts_with("--") => install::install(&client, &args[1..]).await?,
        ["install"] if std::path::Path::new("plugins.lock").exists() => install::locked(&client).await?,
        ["plugins", "check", ..] => install::check(&client, &args[2..]).await?,
        ["plugins", "update", ..] => install::update(&client, &args[2..]).await?,
        _ => command(&client, &args).await?,
    };
    client.close().await;
    Ok(code)
}

fn printed(text: String) -> ExitCode {
    println!("{text}");
    ExitCode::SUCCESS
}

/// Calls the tool `args` name, or shows what they could have named.
async fn command(client: &Client, args: &[String]) -> Result<ExitCode> {
    let (words, flags) = args.split_at(args.iter().take_while(|arg| !arg.starts_with("--")).count());
    let name = words.join("_");
    let (listed, answered) = match client.fresh("tools/list", &json!({})).await {
        Ok(listed) => (listed, Ok(())),
        Err(error) => (client.cached("tools/list", &json!({})).unwrap_or_default(), Err(error)),
    };
    let tools = listed["tools"].as_array().map(Vec::as_slice).unwrap_or_default();

    match tools.iter().find(|tool| tool["name"] == name.as_str()) {
        Some(tool) if flags.iter().any(|flag| flag == "--help") => usage(tool),
        Some(tool) => {
            answered?;
            let inputs = inputs(client, tool, flags).await?;
            let result = client.request("tools/call", json!({"name": name, "arguments": inputs})).await?;
            let parts = result["content"].as_array().map(Vec::as_slice).unwrap_or_default();
            if result["isError"] == true {
                let text: Vec<&str> = parts.iter().filter_map(|part| part["text"].as_str()).collect();
                eprintln!("{}", text.join("\n"));
                return Ok(ExitCode::FAILURE);
            }
            // Text is printed, and a file is written out as it is, to be sent to a file: `> photo.jpg`.
            for part in parts {
                match part["text"].as_str() {
                    Some(text) => println!("{text}"),
                    None => {
                        // A large file is one the client fetched and kept, and any other is in the answer.
                        let file = match part["uri"].as_str().and_then(|uri| uri.strip_prefix("/.file/")) {
                            Some(kept) => client.file(kept).context("the answer isn't a file")?.1,
                            None => {
                                let file = part["data"].as_str().or(part["resource"]["blob"].as_str());
                                BASE64.decode(file.unwrap_or_default().as_bytes()).context("the answer isn't a file")?
                            }
                        };
                        io::stdout().write_all(&file)?;
                    }
                }
            }
        }
        None => {
            let within: Vec<&Value> = tools
                .iter()
                .filter(|tool| {
                    name.is_empty() || tool["name"].as_str().is_some_and(|tool| tool.starts_with(&format!("{name}_")))
                })
                .collect();
            // What a server says now, or what one last said if none is answering.
            let says = async |method: &str, params: Value| match answered {
                Ok(()) => client.fresh(method, &params).await.ok(),
                Err(_) => client.cached(method, &params),
            };
            // A plugin with no tools can still have a skill: the UI library's says how an app is written with it. It's
            // read as any skill is, and what's printed of it is what follows its name and description. A plugin's
            // own skill is named here by the plugin's name, as it's added, and One Core's by its name. Where both
            // are, the plugin's is shown with its tools.
            let skills = says("skills/list", json!({})).await.unwrap_or_default();
            let skills = skills["skills"].as_array().map(Vec::as_slice).unwrap_or_default();
            let uri = skills.iter().filter_map(|skill| skill["uri"].as_str()).rfind(|uri| said(uri) == words);
            let skill = match uri {
                Some(uri) => says("resources/read", json!({"uri": uri})).await,
                None => None,
            };
            let skill = skill.as_ref().and_then(|skill| skill["contents"][0]["text"].as_str());
            let skill = skill.map(|skill| skill.split_once("\n---\n").map_or(skill, |(_, text)| text).trim());
            if within.is_empty() && skill.is_none() {
                answered?;
                bail!("There's no tool, plugin or skill named `{}`. `one --help` lists them.", words.join(" "));
            }
            if answered.is_err() {
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
                println!("\n{THIS_DEVICE}");
                println!("\nA skill says when and how to use a plugin's tools, and `one <skill>` shows one:\n");
                for skill in skills {
                    let words = said(skill["uri"].as_str().unwrap_or_default()).join(" ");
                    let about = skill["frontmatter"]["description"].as_str().unwrap_or_default();
                    println!("one {words:width$}  {about}");
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The words that show a skill, from its address (`OneCore.Skills`): a plugin's own, `skill://<plugin>/<its own
/// name>/SKILL.md`, by the plugin's name; one of its `skills/` by the plugin's name and the skill's; and One Core's
/// own, `skill://<name>/SKILL.md`, by its name.
fn said(uri: &str) -> Vec<&str> {
    let path = uri.strip_prefix("skill://").and_then(|uri| uri.strip_suffix("/SKILL.md")).unwrap_or_default();
    match path.split('/').collect::<Vec<_>>().as_slice() {
        [plugin, _own] => vec![plugin],
        [plugin, .., name] => vec![plugin, name],
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
                let plugin = tool["name"].as_str().and_then(|tool| tool.split('_').next()).unwrap_or_default();
                Value::String(client.upload(plugin, file).await?)
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
/// or `{"base64": ...}` for one that isn't text, as `deploy` takes a plugin's files. A file for an input that says
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
