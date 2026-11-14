//! The CLI. Every tool is a command: the words before the first `--` input, joined by `_`, are the tool's name, so
//! `one tracker add --tracker water` calls `tracker_add`. `one mcp` serves MCP over stdio instead.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, fs, io};

use anyhow::{Context, Result, bail};
use data_encoding::BASE64;
use serde_json::{Map, Value, json};

use crate::{Client, mcp};

pub fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio starts");
    runtime.block_on(run(env::args().skip(1).collect())).unwrap_or_else(|error| {
        eprintln!("{error:#}");
        ExitCode::FAILURE
    })
}

async fn run(args: Vec<String>) -> Result<ExitCode> {
    let client = Arc::new(Client::open(crate::home()?).await?);
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
            let result = client.request("tools/call", json!({"name": name, "arguments": inputs(tool, flags)?})).await?;
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
                        let file = part["data"].as_str().or(part["resource"]["blob"].as_str()).unwrap_or_default();
                        io::stdout().write_all(&BASE64.decode(file.as_bytes()).context("the answer isn't a file")?)?;
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
            // A plugin with no tools can still have a guide: the kit's says how an app is written with it.
            let guide = listed["_meta"]["guides"][&name].as_str();
            if within.is_empty() && guide.is_none() {
                answered?;
                bail!("There's no tool or plugin named `{}`. `one` lists them.", words.join(" "));
            }
            if answered.is_err() {
                eprintln!("No server answered, so this is what one last listed.\n");
            }
            if let Some(guide) = guide {
                println!("{}\n", guide.trim_end());
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
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// A tool's inputs from the command line: `--name value`, typed as the tool says, or `--name-from path` to read
/// the value from a file.
fn inputs(tool: &Value, flags: &[String]) -> Result<Value> {
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

/// A file's text, or a folder as its files' text by their paths in it. Names starting with a dot are left out. A
/// file for an input that says it's base64 (its schema's `contentEncoding`) is read as it is and written as that.
fn read(path: &Path, base64: bool) -> Result<Value> {
    let couldnt = || format!("couldn't read {}", path.display());
    if base64 {
        return Ok(Value::String(BASE64.encode(&fs::read(path).with_context(couldnt)?)));
    }
    if !path.is_dir() {
        return Ok(Value::String(fs::read_to_string(path).with_context(couldnt)?));
    }
    fn files(folder: &Path, within: &str, into: &mut Map<String, Value>) -> Result<()> {
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if entry.file_type()?.is_dir() {
                files(&entry.path(), &format!("{within}{name}/"), into)?;
            } else {
                into.insert(format!("{within}{name}"), read(&entry.path(), false)?);
            }
        }
        Ok(())
    }
    let mut into = Map::new();
    files(path, "", &mut into)?;
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
    println!("\n--<input>-from <path> reads an input from a file, or from a folder as its files.");
}
