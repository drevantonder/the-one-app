//! Apps' tools from the command line and MCP: inputs from flags, words, piped JSON and files, help built from
//! each tool's input schema, and files in results.

mod check;
mod files;
mod help;
mod input;
mod mcp;
mod run;
mod scopes;
mod words;

pub use check::{check, nearest};
pub use words::said;
pub use files::{FILE_LIMIT, as_file, mime, read_file};
pub use help::{forms, list, optional, usage};
pub use mcp::{BUDGET, MCP_FILE_LIMIT, mcp_content, mcp_files, mcp_schema, mcp_tool, within_budget};
pub use input::{Flags, input, split, takes};
pub use run::{Run, dispatch};

use anyhow::Result;
use serde_json::{Value, json};

use crate::api::Client;
use crate::cli::{Failed, asked_yes};
use crate::config::CLI;
use crate::core_tools::Reader;

use files::show;
use help::help;
use input::{from_stdin, missing, places};

/// A tool as the command line reads it, with its set: the core's (`core_tools`), or an app's as `GET /v1/apps`
/// describes it; None when nothing here lists it.
pub fn find(node: &Client, app: &str, tool: &str) -> Option<(Value, Value)> {
    let apps = crate::core_tools::and_apps(node).ok()?;
    let a = apps.iter().find(|a| a["name"] == app)?.clone();
    let o = a["tools"].as_array()?.iter().find(|o| o["name"] == tool)?.clone();
    Some((a, o))
}

/// A command's exit code for what went wrong: 2 what was typed is wrong, 3 refused (it needs --confirm, the user
/// isn't allowed, or isn't signed in), 1 anything else.
pub fn exit_code(err: &anyhow::Error) -> i32 {
    match err.downcast_ref::<Failed>() {
        Some(Failed::Typed(_)) => 2,
        Some(Failed::Refused(_)) => crate::cli::exit::REFUSED,
        None => match crate::protocol::Coded::of(err).as_deref() {
            Some(crate::protocol::code::REFUSED | crate::protocol::code::NOT_SIGNED_IN) => crate::cli::exit::REFUSED,
            _ => 1,
        },
    }
}

fn typed(err: anyhow::Error) -> anyhow::Error {
    match err.downcast::<Failed>() {
        Ok(f) => f.into(),
        Err(e) => Failed::Typed(format!("{e:#}")).into(),
    }
}

/// `<CLI> <app> <tool> [words] [--input value]...`: runs it and prints what it gives back.
pub fn run(g: &crate::cli::Given, app: &str, tool: &str, args: &[String]) -> Result<()> {
    let node = g.node;
    let (mut words, mut flags) = split(args);
    // A last word `-` asks for inputs from stdin even when none is missing; otherwise stdin is read only when one is,
    // so a command inside a `while read` loop doesn't eat the loop's input.
    let asked = words.last().is_some_and(|w| w == "-") && words.pop().is_some();
    let spec = find(node, app, tool);
    let Some((a, o)) = &spec else { return Err(Failed::Typed(unknown(node, app, Some(tool), Some(g.commands))?).into()) };
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
                "missing {}: give {} as flags, or pipe the inputs as JSON: {CLI} {app} {tool} < input.json. See {CLI} {app} {tool} --help.",
                missing.join(", "),
                if missing.len() == 1 { "it" } else { "them" }
            ))
            .into());
        }
    }
    check(&o["input"], &Value::Object(input.clone())).map_err(typed)?;
    // At a terminal, the user is asked here; what they said is checked where every tool runs.
    let confirmed = confirmed || (o["confirm"] == true && asked_yes(&Reader::Cli.named(app, tool))?);
    let how = Run { reader: Reader::Cli, device, scopes, agent: g.agent, confirmed, trace: g.trace };
    let ran = run::run(node, &Reader::Mcp.named(app, tool), o["confirm"] == true, input, how);
    let mut ok = match ran.ok {
        Ok(ok) => ok,
        Err(e) => {
            // The trace matters most when it failed.
            print_trace(ran.trace.as_deref());
            let said = in_cli_words(node, &format!("{e:#}"));
            return Err(match e.downcast::<Failed>() {
                Ok(Failed::Refused(_)) => Failed::Refused(said).into(),
                Ok(Failed::Typed(_)) => Failed::Typed(said).into(),
                Err(e) => crate::protocol::Coded::or_plain(crate::protocol::Coded::of(&e), said),
            });
        }
    };
    if let Some(none) = none_said(node, a, o, &ok, Reader::Cli).filter(|_| !g.json) {
        println!("{none}");
        print_trace(ran.trace.as_deref());
        return Ok(());
    }
    if let Some(text) = ok.as_str() {
        ok = json!(in_cli_words(node, text));
    }
    let shown = show(app, tool, &ok, out.as_deref(), g.json);
    print_trace(ran.trace.as_deref());
    shown
}

/// Prints a call's trace (`--trace`), after its answer, to stderr, so what's on stdout stays the answer.
pub fn print_trace(lines: Option<&[String]>) {
    if let Some(lines) = lines.filter(|l| !l.is_empty()) {
        eprintln!("\ntrace:\n  {}", lines.join("\n  "));
    }
}

/// What an app says, as the command line says it: each tool it names (`gmail_sign_in`) as its command
/// (`one gmail sign_in`), as it was when the node can't list its apps: the error that came with it matters more.
pub fn in_cli_words(node: &Client, text: &str) -> String {
    let apps = crate::core_tools::and_apps(node).unwrap_or_default();
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

/// An empty list of accounts, from a tool that says it lists them (`accounts: true`), in words, with what to do:
/// screens want the list, a user or agent reading wants this.
pub fn none_said(node: &Client, app: &Value, tool: &Value, ok: &Value, reader: Reader) -> Option<String> {
    if tool["accounts"] != true || ok.as_array().is_none_or(|a| !a.is_empty()) {
        return None;
    }
    let (name, title) = (app["name"].as_str().unwrap_or(""), app["title"].as_str().unwrap_or(""));
    let signs_in = app["tools"].as_array().into_iter().flatten().any(|o| o["name"] == "sign_in");
    // One that needs a part on a device, with none of yours running it: signing in can't work yet.
    let parts: Vec<&Value> = app["uses"]["services"].as_array().into_iter().flatten().collect();
    let on_none = || {
        let state = node.state().ok()?;
        Some(!state.network?.devices.iter().any(|d| d.now.services.iter().any(|s| parts.iter().any(|p| *p == s))))
    };
    Some(match (signs_in, app["needs"].as_str()) {
        (_, Some(needs)) if !parts.is_empty() && on_none() == Some(true) => format!("No {title} accounts: {title} isn't on any of your devices yet. {needs}."),
        (true, _) => format!("No account is signed in to {title} yet: {} signs one in.", reader.named(name, "sign_in")),
        (false, Some(needs)) => format!("No {title} accounts: {title} isn't on any of your devices yet. {needs}."),
        (false, None) => format!("No {title} accounts yet."),
    })
}

/// What to say for an app or tool nobody here lists, with the nearest names: `gmail has no tool serch:
/// did you mean search?`, `no app gmial: did you mean gmail?`. On the command line, built-in commands are names too.
/// `commands` are the command line's own words, when it asks.
pub fn unknown(node: &Client, app: &str, tool: Option<&str>, commands: Option<&[String]>) -> Result<String> {
    let cli = commands.is_some();
    let apps = crate::core_tools::and_apps(node)?;
    let tools_of = |a: &Value| -> Vec<String> { a["tools"].as_array().into_iter().flatten().filter_map(|o| o["name"].as_str()).filter(|o| !o.starts_with('_')).map(String::from).collect() };
    // One the user could add: the node lists it, without tools for them, since it isn't added where they are.
    let addable = |a: &Value| a["tools"].is_null() && a["deleted"].is_null();
    if let Some(a) = apps.iter().find(|a| a["name"] == app) {
        let title = a["title"].as_str().unwrap_or(app);
        if !a["deleted"].is_null() {
            return Ok(match cli {
                true => format!("{title} was deleted: {CLI} apps restore {app} brings it back, for its owners"),
                false => format!("{title} was deleted: apps_restore with app {app} brings it back, for its owners"),
            });
        }
        if addable(a) {
            return Ok(match cli {
                true => format!("{title} isn't added yet: {CLI} add {app} adds it for you, or {CLI} add {app} --scope global for everyone"),
                false => format!("{title} isn't added yet: add with app {app} and scope user adds it for the user, or global for everyone"),
            });
        }
        let (tool, tools) = (tool.unwrap_or(""), tools_of(a));
        let guess = nearest(tool, &tools.iter().map(String::as_str).collect::<Vec<_>>()).first().map(|n| format!(": did you mean {n}?")).unwrap_or_else(|| ".".into());
        return Ok(format!("{app} has no tool {tool}{guess} Its tools: {}", tools.join(", ")));
    }
    // A tool's name as a command, one repos_create notes: the command is one repos create notes.
    let as_command = app.match_indices('_').find_map(|(i, _)| {
        let (a, o) = (&app[..i], &app[i + 1..]);
        apps.iter().find(|x| x["name"] == a).is_some_and(|x| tools_of(x).iter().any(|x| x == o)).then(|| format!("{a} {o}"))
    });
    if let Some(command) = as_command.filter(|_| cli) {
        return Ok(format!("no app {app}: did you mean {CLI} {command}{}?", tool.map(|o| format!(" {o}")).unwrap_or_default()));
    }
    let names: Vec<String> = apps.iter().filter(|a| a["deleted"].is_null()).filter_map(|a| a["name"].as_str()).map(String::from).collect();
    let commands = commands.unwrap_or_default();
    let all: Vec<&str> = names.iter().chain(commands.iter()).map(String::as_str).collect();
    Ok(match nearest(app, &all).first() {
        Some(n) if *n == app && commands.contains(n) => format!("{CLI} {app} takes more than that: {CLI} {app} --help says what"),
        Some(n) if commands.contains(n) && !names.contains(n) => format!("no command {app}: did you mean {CLI} {n}? {CLI} help says the rest"),
        Some(n) => format!("no app {app}: did you mean {n}?"),
        None if cli => format!("no command or app {app}: {CLI} help says the commands, {CLI} search <words> finds tools"),
        None => format!("no app {app}: search finds the tools for a task"),
    })
}

/// Whether a tool takes a `device` of its own (an agent device's name, say): then that's what `device` is, and
/// it says itself where it runs, so the CLI's and MCP's `device` (which of your devices runs it) isn't offered.
pub fn own_device(tool: &Value) -> bool {
    tool["input"]["properties"].get("device").is_some()
}

/// Whether an app's calls run on a device of the user's choosing: it runs on every device, or uses a service (Shell
/// on a named device, iMessage on the Mac), so MCP and search offer `device` as the CLI's `--device` does.
pub(crate) fn runs_on_devices(app: &Value) -> bool {
    app["runs"] == "here" || app["uses"]["services"].as_array().is_some_and(|s| !s.is_empty())
}

/// What `device` says it does: which device runs the call, and which when none is given.
fn device_about(app: &Value) -> &'static str {
    match (app["runs"] == "here", runs_on_devices(app)) {
        (true, _) => "which of your devices runs it, by name; all for every one; default is this one",
        (_, true) => "which of your devices runs it, by name; default is the first online one that can, in Device Activity's order",
        _ => "run it on that device (by name)",
    }
}

/// Any app's tools on the command line, after every built-in command.
pub fn commands() -> crate::cli::Command {
    crate::cli::Command {
        help: vec![
            ("<app>".into(), "an app's tools".into()),
            ("<app> <tool> [words] [--input value]".into(), format!("run a tool, e.g. {CLI} repos list; --help says what it takes. Words fill its main inputs in order (--help shows which), lists repeat a flag, required inputs not given are asked for at a terminal, else read as JSON from stdin (a last word - reads it anyway), --file <input>=<path> gives a file and --out <path> saves what it gives back. --device <name> runs it on that device; --trace prints, after its answer (or why it failed), what it did: the devices tried, the one that ran it, each wait in ms, where its data came from, and the total")),
        ],
        run: |g| {
            let args = g.args;
            match g.words().as_slice() {
                [app] if !app.starts_with('-') => {
                    let apps = crate::core_tools::and_apps(g.node)?;
                    let Some(found) = apps.iter().find(|a| a["name"] == *app) else { anyhow::bail!("{}", unknown(g.node, app, None, Some(g.commands))?) };
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
                [first, ..] => anyhow::bail!("{}", unknown(g.node, first, None, Some(g.commands))?),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}
