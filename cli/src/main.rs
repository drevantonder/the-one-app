//! `one`, what the agents of a network's users and admins run: each feature's commands, asked of this machine's node over
//! its API. Run as `<cli>d` it's the node (src/oned.rs), and as `git-remote-<cli>` git's remote helper (git_remote.rs).


use anyhow::Result;
use ::custom_build::config::{CLI, NAME, YOURS};
use api::cli::{Command, Given};
use serde_json::Value;

mod code;
mod git_remote;
mod mcp;
mod network;
mod skills;
mod tools;

use api::Client;
use api::{Reader, SENDS};

/// Every built-in command, each feature's, in the order they're tried: the first that takes what was typed runs it.
fn commands() -> Vec<Command> {
    use one::node::{apps, blobs, storage};
    vec![
        // First: it must run with nothing else of this program working.
        one::update::cli::commands(),
        network::commands(),
        groups::cli::commands(),
        backups_runtime::commands(),
        devices::cli::commands(),
        apps::cli::commands(),
        vault::cli::commands(),
        one::cloudflare::cli::commands(),
        storage::cli::commands(),
        services::cli::commands(),
        device_logs::cli::commands(),
        insights::cli::commands(),
        one::build::commands(),
        blobs::cli::commands(),
        own(),
        skills::commands(),
        code::commands(),
        mcp::commands(),
        tools::commands(),
    ]
}

/// Every built-in command, with what it takes, then the network's own tools as commands: `help all`.
fn every_command(node: &Client) -> String {
    let core = core_forms(node);
    format!(
        "Usage:\n{}\n{NAME}'s own tools, each a command as an app's tools are (--help after one says what it takes):\n{}\n{NAME} holds one network. For another context, run that context's own app.",
        api::cli::help(commands().iter().flat_map(|c| &c.help)),
        api::cli::help(&core)
    )
}

/// The core's tools as commands' forms, when a node can say what they are.
fn core_forms(node: &Client) -> Vec<(String, String)> {
    let core: Vec<Value> = one::tools::sets(node).unwrap_or_default().into_iter().filter(|s| s["runs"] == "client").collect();
    one::tools::forms(&core)
}

/// What `help` and the bare command say: what an agent needs to start, and where the rest is.
fn help(node: &Client) -> String {
    let apps = node.apps().unwrap_or_default();
    let names: Vec<&str> = apps.iter().filter(|a| a["tools"].as_array().is_some_and(|o| !o.is_empty())).filter_map(|a| a["name"].as_str()).collect();
    let yours = match names.is_empty() {
        true => String::new(),
        false => format!("\nYour apps: {}.\n", names.join(", ")),
    };
    format!(
        "{NAME}: your devices, and your team's, as one place for apps and agents.

  {CLI} search <words>         the tools for a task in every app, each with how to call it
  {CLI} <app>                  an app's tools and its guide
  {CLI} <app> <tool> --help    what a tool takes
  {CLI} <app> <tool> [words] [--<input> <value>]
                             run a tool, like {CLI} repos list
  {CLI} run <file>             a script that calls several tools; {CLI} search gives each call
  {CLI} skills                 the guides agents load
  {CLI} status                 who you are, and your devices
  {CLI} mcp [--code]           the local MCP server, for agents
{yours}
A tool's words fill its main inputs in order, a list repeats its flag, and inputs not given are read as JSON
from stdin. --file <input>=<path> gives a file, --out <path> saves what comes back, --device <name> runs it there,
and --confirm goes ahead with what {SENDS}, once the user agreed. --json prints what comes back
as JSON. --trace prints what the call did, and where, after its answer: use it, and {CLI} logs <device>, before ssh.

Exit codes: 0 done, 2 what was typed is wrong, 3 refused (needs --confirm, not allowed, or not signed in), 1 anything
else. Data goes to stdout, messages to stderr.

{CLI} help all lists every command; {CLI} <command> --help says what one takes."
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if one::oned::invoked() {
        if let Err(err) = one::oned::run(&args) {
            eprintln!("{}: {err:#}", ::custom_build::programs::oned());
            std::process::exit(1);
        }
        return;
    }
    if onet::computer::invoked() {
        if let Err(err) = onet::computer::run(&args) {
            eprintln!("{}: {err:#}", onet::computer::name());
            std::process::exit(1);
        }
        return;
    }
    // Run by git, for a repo's address.
    if platform::os::invoked_as().is_some_and(|me| me == ::custom_build::programs::git_helper()) {
        match git_remote::run(&args) {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!("{CLI}: {err:#}");
                std::process::exit(128);
            }
        }
    }
    if let Err(err) = run(args) {
        // What the node says names its tools (apps_restore); the command line names them as commands.
        eprintln!("{CLI}: {}", tools::in_cli_words(&node(Reader::Cli.client()), &format!("{err:#}")));
        std::process::exit(api::cli::exit_code(&err));
    }
}

fn run(args: Vec<String>) -> Result<()> {
    // --json is everyone's, wherever it's typed: what a tool gives back, as JSON.
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let json = args[..end].iter().any(|a| a == "--json");
    // --trace too: a call prints what it did, and where, after its answer.
    let trace = args[..end].iter().any(|a| a == "--trace");
    let args: Vec<String> = args.iter().enumerate().filter(|(i, a)| *i >= end || (*a != "--json" && *a != "--trace")).map(|(_, a)| a.clone()).collect();
    let (positional, flags) = api::tools::split(&args);
    if positional.first().map(String::as_str) != Some("mcp") {
        // Printing into `| head` ends quietly, as other commands do, instead of panicking.
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL)
        };
    }
    let client = if positional.first().map(String::as_str) == Some("mcp") { Reader::Mcp } else { Reader::Cli };
    let (agent, node, forms) = (mcp::agent_from_env(), node(client.client()), commands().into_iter().flat_map(|c| c.help).collect::<Vec<_>>());
    let given = Given { words: &positional, flags: &flags, args: &args, node: &node, agent: agent.as_deref(), json, trace, commands: &forms };
    // --help shows help, whatever the command: never runs it. An app's tool's is the app's own, and with no command
    // it's what the bare command says (`own`).
    if !positional.is_empty() && (flags.contains_key("help") || flags.contains_key("h")) {
        match command_help(&node, &positional) {
            Some(text) => print!("{text}"),
            None => _ = (tools::commands().run)(&given)?,
        }
        return Ok(());
    }
    for command in commands() {
        if (command.run)(&given)? {
            return Ok(());
        }
    }
    unreachable!("any word is an app's name, if not a command's")
}

/// This build, and help: what to start with, and every command.
fn own() -> Command {
    Command {
        help: vec![("version".into(), "this build".into()), ("help [all]".into(), "what to start with; with all, this".into())],
        run: |g| {
            match g.words().as_slice() {
                [] | ["help"] => {
                    println!("{}", help(g.node));
                    println!("\n{}", connection(g.node));
                }
                ["help", "all"] => println!("{}", every_command(g.node)),
                ["help", rest @ ..] => match command_help(g.node, &rest.iter().map(|w| w.to_string()).collect::<Vec<_>>()) {
                    Some(text) => print!("{text}"),
                    None => return Err(api::cli::Failed::Typed(one::tools::unknown(g.node, rest[0], None, Some(g.commands))?).into()),
                },
                ["version"] if g.json => println!("{}", one::release::Version::this().json()),
                ["version"] => println!("{} {} ({})", NAME, ::custom_build::config::BUILD, ::custom_build::config::TARGET),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// A built-in command's help, for --help after it: its forms, as `help all` has them. None for a tool, for an app that
/// isn't a built-in command too, whose help is their own, and for what's neither, which says what was mistyped.
fn command_help(node: &Client, words: &[String]) -> Option<String> {
    words.first()?;
    let typed: Vec<&str> = words.iter().take_while(|w| !w.starts_with('-')).map(String::as_str).collect();
    if let [set, tool, ..] = typed[..]
        && one::tools::find(node, set, tool).is_some()
    {
        return None;
    }
    let (commands, core) = (commands(), core_forms(node));
    let forms: Vec<&(String, String)> = commands.iter().flat_map(|c| &c.help).chain(&core).collect();
    // The forms for the most words given that have any: vault answer's, else every vault form.
    let starts = |n: usize| -> Vec<&(String, String)> { forms.iter().copied().filter(|(usage, _)| usage.split_whitespace().take(n).eq(typed[..n].iter().copied())).collect() };
    let these = (1..=typed.len()).rev().map(starts).find(|f| !f.is_empty()).unwrap_or_default();
    (!these.is_empty()).then(|| format!("Usage:\n{}", api::cli::help(these)))
}

fn connection(node: &Client) -> String {
    let state = match node.state() {
        Ok(state) => state,
        Err(err) => return format!("{err:#}"),
    };
    let device = &state.device.name;
    let Some(network) = &state.network else {
        return format!("{}.", one::node::not_joined(device));
    };
    let network = if network.name.is_empty() { format!("{YOURS} (still joining)") } else { network.name.clone() };
    let ready = if state.device.ready { "" } else { ", catching up" };
    format!("You're {} in {network}, on {device}{ready}.", state.looking())
}

/// How the CLI asks this machine's onet, as one of its own programs: again while it starts again (`one::loopback`),
/// once it has run here, which made its token.
struct Http;

impl api::client::Via for Http {
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, wait: Option<std::time::Duration>) -> Result<api::client::Answer> {
        let body = body.unwrap_or_default();
        let ask = || {
            let mut req = ureq::http::Request::builder().method(method).uri(format!("http://{}{path}", api::local::addr()));
            for (name, value) in headers {
                req = req.header(*name, *value);
            }
            let agent = api::local::agent();
            let req = agent.configure_request(req.body(body.clone())?).timeout_global(wait).build();
            agent.run(req)
        };
        let asked = || if api::local::local_token(&platform::home()).is_some() { one::loopback::while_refused(&ask) } else { ask() };
        let resp = match asked() {
            Ok(resp) => resp,
            Err(err) if one::loopback::lost(&err) => {
                let apps = || self.send("GET", ::api::path::APPS, &[], None, None).ok().and_then(|a| serde_json::from_reader(a.body).ok());
                anyhow::ensure!(one::loopback::reads(method, path, &body, apps), "{}", one::loopback::lost_write());
                asked().map_err(unreached)?
            }
            Err(err) => return Err(unreached(err)),
        };
        let trace = resp.headers().get(::custom_build::protocol::header::TRACE).and_then(|h| h.to_str().ok()).map(str::to_string);
        Ok(api::client::Answer { status: resp.status().as_u16(), trace, body: Box::new(resp.into_body().into_reader()) })
    }
}

/// The node, as `client` asks it (`Reader::client`): the CLI, or its MCP server.
fn node(client: &'static str) -> Client {
    Client::new(Http, false, client)
}

/// The one thing to say when nothing answers: what to install or start.
fn unreached(err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            let start = match () {
                _ if cfg!(target_os = "macos") => "open its app".to_string(),
                _ if cfg!(windows) => format!("Start-ScheduledTask {CLI}d, in PowerShell"),
                _ => format!("sudo systemctl start {CLI}d"),
            };
            anyhow::anyhow!("{NAME} isn't running on this computer: install it here, or if it's installed, start it ({start})")
        }
        err => anyhow::anyhow!("{err}"),
    }
}
