//! `search` and `run` on the command line: the tools for a task, and a script that calls several (`api_features::code`).

use anyhow::{Context, bail};
use serde_json::Value;

use api::Reader;
use custom_build::config::CLI;
use api_features::code::{catalogs, execute, search, shown};

/// `search` and `run` on the command line.
pub fn commands() -> ::api::cli::Command {
    ::api::cli::Command {
        help: vec![
            ("search <words>".into(), "the tools for a task in every app: how to call each, and its app's guide".into()),
            ("run <file>".into(), format!("run a short script that calls tools, as tools.<app>.<tool>({{ ... }}) (- reads it from stdin), and print what it returns; {CLI} search gives each call")),
        ],
        run: |g| {
            match g.words().as_slice() {
                ["run", file] => {
                    let script = match *file {
                        "-" => std::io::read_to_string(std::io::stdin())?,
                        file => std::fs::read_to_string(file).with_context(|| format!("read {file}"))?,
                    };
                    let result = execute(g.node, &script, g.agent)?;
                    for line in result["logs"].as_array().into_iter().flatten() {
                        eprintln!("{}", line.as_str().map(String::from).unwrap_or_else(|| line.to_string()));
                    }
                    match result["ok"] == true {
                        true => println!("{}", match &result["value"] {
                            Value::String(s) => s.clone(),
                            v if std::io::IsTerminal::is_terminal(&std::io::stdout()) => serde_json::to_string_pretty(v)?,
                            v => serde_json::to_string(v)?,
                        }),
                        false => bail!("{}", shown(&result)),
                    }
                }
                ["search", words @ ..] if !words.is_empty() => {
                    let limit = match (g.has("limit"), g.flag("limit")) {
                        (false, _) => 8,
                        (true, given) => given.and_then(|l| l.parse().ok()).ok_or_else(|| ::api::cli::Failed::Typed(format!("--limit is how many tools, a number: not {}", given.unwrap_or("nothing"))))?,
                    };
                    let (apps, addable) = catalogs(g.node)?;
                    println!("{}", search(&apps, &addable, &words.join(" "), limit, Reader::Cli));
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}
