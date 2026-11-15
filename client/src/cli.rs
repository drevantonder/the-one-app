//! The CLI. Every tool is a command: the words before the first `--` input, joined by `_`, are the tool's name, so
//! `one notes add --text milk` calls `notes_add`. `one notes` shows the plugin's skill and its tools, and
//! `one mcp` serves MCP over stdio instead.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::{env, fs, io};

use anyhow::{Context, Result, bail};
use data_encoding::BASE64;
use serde_json::{Map, Value, json};

use crate::{Client, NotIn, mcp};

pub fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio starts");
    runtime.block_on(run(env::args().skip(1).collect())).unwrap_or_else(|error| {
        eprintln!("{error:#}");
        ExitCode::FAILURE
    })
}

async fn run(args: Vec<String>) -> Result<ExitCode> {
    // A device that isn't in its network yet says its code and where it's let in, and carries on once it is.
    let client = match Client::open(crate::home()?).await {
        Err(error) if error.is::<NotIn>() => {
            let not_in = error.downcast::<NotIn>()?;
            eprintln!("{not_in}");
            not_in.answered().await?;
            Client::open(crate::home()?).await?
        }
        opened => opened?,
    };
    let code = if args == ["mcp"] {
        mcp::serve(client.clone()).await?;
        ExitCode::SUCCESS
    } else {
        command(&client, &args).await?
    };
    client.close().await;
    Ok(code)
}

/// Calls the tool `args` name, or shows what they could have named.
async fn command(client: &Client, args: &[String]) -> Result<ExitCode> {
    let (words, flags) = args.split_at(args.iter().take_while(|arg| !arg.starts_with("--")).count());
    let name = words.join("_");
    let (listed, answered) = match client.fresh("tools/list", &json!({})).await {
        Ok(listed) => (listed, Ok(())),
        Err(error) => (client.kept("tools/list", &json!({})).unwrap_or_default(), Err(error)),
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
                Err(_) => client.kept(method, &params),
            };
            // A plugin with no tools can still have a skill: the kit's says how an app is written with it. It's
            // read as any skill is, and what's printed of it is what follows its name and description. A plugin's
            // own skill is named here by the plugin's name, as it's added, and One Core's by its name. Where both
            // are, the plugin's is shown with its tools.
            let skills = says("skills/list", json!({})).await.unwrap_or_default();
            let skills = skills["skills"].as_array().map(Vec::as_slice).unwrap_or_default();
            let uri = skills.iter().filter_map(|skill| skill["uri"].as_str()).filter(|uri| said(uri) == words).last();
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

/// A tool's inputs from the command line: `--name value`, typed as the tool says, or `--name-from path` to read
/// the value from a file. A file for an input that says it's an upload (its schema's `format`) is sent to where
/// the network keeps it, and the input is the name it was given there.
async fn inputs(client: &Client, tool: &Value, flags: &[String]) -> Result<Value> {
    let known = &tool["inputSchema"]["properties"];
    let mut inputs = Map::new();
    let mut flags = flags.iter().peekable();
    while let Some(flag) = flags.next() {
        let name = flag.strip_prefix("--").with_context(|| format!("`{flag}` isn't an input: write --name value"))?;
        let (name, from_file) = match name.strip_suffix("-from") {
            Some(name) if known.get(name).is_some() => (name, true),
            _ => (name, false),
        };
        let kind =
            known.get(name).with_context(|| format!("It has no input named `{name}`. --help shows them."))?["type"]
                .as_str()
                .unwrap_or("JSON");
        let value = match flags.next_if(|next| !next.starts_with("--")) {
            None if kind == "boolean" => Value::Bool(true),
            None => bail!("--{name} needs a value"),
            Some(path) if from_file && known[name]["format"] == "upload" => {
                let file = fs::read(path).with_context(|| format!("couldn't read {path}"))?;
                let plugin = tool["name"].as_str().and_then(|tool| tool.split('_').next()).unwrap_or_default();
                Value::String(client.upload(plugin, file).await?)
            }
            Some(path) if from_file => read(Path::new(path), known[name]["contentEncoding"] == "base64")?,
            Some(text) if kind == "string" => Value::String(text.clone()),
            Some(text) => {
                serde_json::from_str(text).with_context(|| format!("--{name} takes {kind}, written as JSON"))?
            }
        };
        inputs.insert(name.to_owned(), value);
    }
    Ok(Value::Object(inputs))
}

/// A file's text, or a folder as its files' text by their paths in it, as `crate::files` reads a folder. A file
/// for an input that says it's base64 (its schema's `contentEncoding`) is read as it is and written as that.
fn read(path: &Path, base64: bool) -> Result<Value> {
    let couldnt = || format!("couldn't read {}", path.display());
    if base64 {
        return Ok(Value::String(BASE64.encode(&fs::read(path).with_context(couldnt)?)));
    }
    if !path.is_dir() {
        return Ok(Value::String(fs::read_to_string(path).with_context(couldnt)?));
    }
    let mut into = Map::new();
    for file in crate::files(path).with_context(couldnt)? {
        let text = read(&path.join(&file), false)?;
        into.insert(file, text);
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
        // One of a few is shown as the few.
        let one_of: Vec<&str> = input["enum"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let kind =
            if one_of.is_empty() { input["type"].as_str().unwrap_or("JSON").to_owned() } else { one_of.join("|") };
        println!("  --{name} <{kind}{optional}>  {about}");
    }
    println!("\n--<input>-from <path> reads an input from a file, or from a folder's files that git doesn't ignore.");
}
