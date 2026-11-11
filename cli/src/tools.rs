//! Any app's tool as a command: its inputs from words, flags, files and piped JSON, what it gives back printed, and what
//! went wrong said in the command line's words.

use anyhow::Result;
use serde_json::{Value, json};

use api::cli::{Failed, asked_yes};
use api::tools::{check, from_stdin, input, missing, places, show, split};
use api::{Client, Reader};
use custom_build::config::CLI;
use ::mcp::tools::{Run, find, help, list, none_said, own_device, scopes, sets, unknown, usage};

/// Any app's tools on the command line, after every built-in command.
pub fn commands() -> ::api::cli::Command {
    ::api::cli::Command {
        help: vec![
            ("<app>".into(), "an app's tools".into()),
            ("<app> <tool> [words] [--<input> <value>]".into(), format!("run a tool, e.g. {CLI} repos list; --help says what it takes. Words fill its main inputs in order (--help shows which), lists repeat a flag, required inputs not given are asked for at a terminal, else read as JSON from stdin (a last word - reads it anyway), --file <input>=<path> gives a file and --out <path> saves what it gives back. --device <name> runs it on that device; --trace prints, after its answer (or why it failed), what it did: the devices tried, the one that ran it, each wait in ms, where its data came from, and the total")),
        ],
        run: |g| {
            let args = g.args;
            match g.words().as_slice() {
                [app] if !app.starts_with('-') => {
                    let apps = sets(g.api)?;
                    let Some(found) = apps.iter().find(|a| a["name"] == *app) else { return Err(Failed::Typed(unknown(g.api, app, None, Some(g.commands))?).into()) };
                    println!("{} ({app})", found["title"].as_str().unwrap_or(app));
                    if let Some(i) = found["instructions"].as_str().filter(|i| !i.is_empty()) {
                        println!("\n{i}\n");
                    }
                    list(std::slice::from_ref(found)).iter().for_each(|l| println!("{l}"));
                    println!("\n{CLI} {app} <tool> --help says what a tool takes.");
                }
                [app, tool, ..] => {
                    let at = |word: &str, from: usize| args.iter().skip(from).position(|a| a == word).map(|i| i + from + 1).unwrap_or(from);
                    run(g, app, tool, &args[at(tool, at(app, 0))..])?
                }
                [first, ..] => return Err(Failed::Typed(unknown(g.api, first, None, Some(g.commands))?).into()),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// `<CLI> <app> <tool> [words] [--<input> <value>]...`: runs it and prints what it gives back.
pub fn run(g: &::api::cli::Given, app: &str, tool: &str, args: &[String]) -> Result<()> {
    let onet = g.api;
    let (mut words, mut flags) = split(args);
    // A last word `-` asks for inputs from stdin even when none is missing; otherwise stdin is read only when one is,
    // so a command inside a `while read` loop doesn't eat the loop's input.
    let asked = words.last().is_some_and(|w| w == "-") && words.pop().is_some();
    let spec = find(onet, app, tool);
    let Some((a, o)) = &spec else { return Err(Failed::Typed(unknown(onet, app, Some(tool), Some(g.commands))?).into()) };
    let call = a["runs"] != "client";
    if flags.contains_key("help") || flags.contains_key("h") {
        print!("{}", help(a, o, call));
        return Ok(());
    }
    let scopes = match call {
        true => flags.remove("scopes").map(|given| scopes::read(given.iter().flatten().map(String::as_str))).transpose().map_err(typed)?,
        false => None,
    };
    let mut take = |k: &str| flags.remove(k).and_then(|v| v.into_iter().last().flatten());
    let device = if call && !own_device(o) { take("device") } else { None };
    let out = take("out");
    let confirmed = flags.remove("confirm").is_some();
    let files: Vec<String> = flags.remove("file").unwrap_or_default().into_iter().flatten().collect();
    let schema = o["input"].clone();
    let places = places(o);
    let mut input = input(&schema, &places, words, flags, &files).map_err(typed)?;
    let missing = missing(&schema, &input);
    if !missing.is_empty() || asked {
        from_stdin(&schema, &missing, &mut input).map_err(typed)?;
        let missing = self::missing(&schema, &input);
        if !missing.is_empty() {
            return Err(Failed::Typed(format!(
                "missing {}: {}. Inputs can also come as JSON on stdin; --help says more.",
                missing.join(", "),
                usage(app, o)
            ))
            .into());
        }
    }
    check(&o["input"], &Value::Object(input.clone())).map_err(typed)?;
    // At a terminal, the user is asked here; what they said is checked where every tool runs.
    let confirmed = confirmed || (o["confirm"] == true && asked_yes(&Reader::Cli.named(app, tool))?);
    let how = Run { reader: Reader::Cli, device, scopes, agent: g.agent, confirmed, trace: g.trace };
    let ran = ::mcp::tools::run(onet, &Reader::Mcp.named(app, tool), o, input, how);
    let mut ok = match ran.ok {
        Ok(ok) => ok,
        Err(e) => {
            // The trace matters most when it failed.
            ::api::cli::print_trace(ran.trace.as_deref());
            return Err(in_cli_error(onet, e));
        }
    };
    if let Some(none) = none_said(onet, a, o, &ok, Reader::Cli).filter(|_| !g.json) {
        println!("{none}");
        ::api::cli::print_trace(ran.trace.as_deref());
        return Ok(());
    }
    if let Some(text) = ok.as_str() {
        ok = json!(in_cli_words(onet, text));
    }
    let shown = show(app, tool, &ok, out.as_deref(), g.json);
    ::api::cli::print_trace(ran.trace.as_deref());
    shown
}

fn typed(err: anyhow::Error) -> anyhow::Error {
    match err.downcast::<Failed>() {
        Ok(f) => f.into(),
        Err(e) => Failed::Typed(format!("{e:#}")).into(),
    }
}

/// A failure of a call, as the command line says it (`in_cli_words`), keeping what kind it is, which its exit code
/// is made from.
pub fn in_cli_error(onet: &Client, e: anyhow::Error) -> anyhow::Error {
    let said = in_cli_words(onet, &format!("{e:#}"));
    match e.downcast::<Failed>() {
        Ok(Failed::Refused(_)) => Failed::Refused(said).into(),
        Ok(Failed::Typed(_)) => Failed::Typed(said).into(),
        Err(e) => ::custom_build::protocol::Coded::or_plain(::custom_build::protocol::Coded::of(&e), said),
    }
}

/// What an app says, as the command line says it: each tool it names (`gmail_sign_in`) as its command
/// (`one gmail sign_in`), as it was when onet can't list its apps: the error that came with it matters more.
pub fn in_cli_words(onet: &Client, text: &str) -> String {
    let apps = sets(onet).unwrap_or_default();
    let command = |word: &str| -> Option<String> {
        word.match_indices('_').find_map(|(i, _)| {
            let (app, tool) = (&word[..i], &word[i + 1..]);
            let a = apps.iter().find(|a| a["name"] == app)?;
            a["tools"].as_array()?.iter().any(|o| o["name"] == tool).then(|| format!("{CLI} {app} {tool}"))
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
