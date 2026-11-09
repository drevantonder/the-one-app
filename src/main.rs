
use anyhow::Result;
use one::config::{CLI, NAME, YOURS};
use one::cli::{Command, Given};
use one::api;
use serde_json::Value;

mod client_device;
mod git_remote;
mod network;
#[path = "onboarding/sign_in.rs"]
mod sign_in;

use one::api::Client;
use one::tools;
use one::core_tools::SENDS;

/// Every built-in command, each feature's, in the order they're tried: the first that takes what was typed runs it.
fn commands() -> Vec<Command> {
    use one::node::{apps, blobs, services, storage, vault};
    vec![
        network::commands(),
        one::groups::cli::commands(),
        one::backups::cli::commands(),
        one::devices::cli::commands(),
        apps::cli::commands(),
        vault::cli::commands(),
        one::cloudflare::cli::commands(),
        storage::cli::commands(),
        services::cli::commands(),
        one::logs::cli::commands(),
        one::build::commands(),
        blobs::cli::commands(),
        own(),
        one::skills::commands(),
        one::code::commands(),
        one::mcp::commands(),
        one::tools::commands(),
    ]
}

/// Every built-in command, with what it takes, then the network's own tools as commands: `help all`.
fn every_command() -> String {
    let core = tools::forms(&one::core_tools::listed());
    format!(
        "Usage:\n{}\n{NAME}'s own tools, each a command as an app's tools are (--help after one says what it takes):\n{}\n{NAME} holds one network. For another context, run that context's own app.",
        one::cli::help(commands().iter().flat_map(|c| &c.help)),
        one::cli::help(&core)
    )
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
            eprintln!("{}: {err:#}", one::oned::name());
            std::process::exit(1);
        }
        return;
    }
    // Run by git, for a repo's address.
    if std::env::args_os().next().is_some_and(|me| std::path::Path::new(&me).file_name().is_some_and(|n| *n == *one::git_wire::helper())) {
        let ran = git_remote::run(&args);
        client_device::end();
        match ran {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!("{CLI}: {err:#}");
                std::process::exit(128);
            }
        }
    }
    let ran = run(args);
    client_device::end();
    if let Err(err) = ran {
        // What the node says names its tools (apps_restore); the command line names them as commands.
        eprintln!("{CLI}: {}", tools::in_cli_words(&node(), &format!("{err:#}")));
        std::process::exit(tools::exit_code(&err));
    }
}

fn run(args: Vec<String>) -> Result<()> {
    // --json is everyone's, wherever it's typed: what a tool gives back, as JSON.
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let json = args[..end].iter().any(|a| a == "--json");
    // --trace too: a call prints what it did, and where, after its answer.
    let trace = args[..end].iter().any(|a| a == "--trace");
    let args: Vec<String> = args.iter().enumerate().filter(|(i, a)| *i >= end || (*a != "--json" && *a != "--trace")).map(|(_, a)| a.clone()).collect();
    let (positional, flags) = tools::split(&args);
    if positional.first().map(String::as_str) != Some("mcp") {
        // Printing into `| head` ends quietly, as other commands do, instead of panicking.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    let (agent, node, forms) = (one::mcp::agent_from_env(), node(), commands().into_iter().flat_map(|c| c.help).collect::<Vec<_>>());
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
                ["help", "all"] => println!("{}", every_command()),
                ["help", rest @ ..] => match command_help(g.node, &rest.iter().map(|w| w.to_string()).collect::<Vec<_>>()) {
                    Some(text) => print!("{text}"),
                    None => return Err(one::cli::Failed::Typed(tools::unknown(g.node, rest[0], None, Some(g.commands))?).into()),
                },
                ["version"] if g.json => println!("{}", one::release::Version::this().json()),
                ["version"] => println!("{} {} ({})", NAME, one::config::BUILD, one::config::TARGET),
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
        && tools::find(node, set, tool).is_some()
    {
        return None;
    }
    let (commands, core) = (commands(), tools::forms(&one::core_tools::listed()));
    let forms: Vec<&(String, String)> = commands.iter().flat_map(|c| &c.help).chain(&core).collect();
    // The forms for the most words given that have any: vault answer's, else every vault form.
    let starts = |n: usize| -> Vec<&(String, String)> { forms.iter().copied().filter(|(usage, _)| usage.split_whitespace().take(n).eq(typed[..n].iter().copied())).collect() };
    let these = (1..=typed.len()).rev().map(starts).find(|f| !f.is_empty()).unwrap_or_default();
    (!these.is_empty()).then(|| format!("Usage:\n{}", one::cli::help(these)))
}

fn connection(node: &Client) -> String {
    let state = match node.state() {
        Ok(state) => state,
        Err(err) if client_device::active() => return format!("This computer is one of your devices, but {err:#}."),
        Err(_) => return format!("{}.", client_device::not_here()),
    };
    if let Some(said) = client_device::said(&state) {
        return format!("{said}.");
    }
    let device = &state.device.name;
    let Some(network) = &state.network else {
        return format!("{device} isn't in {YOURS} yet: {CLI} join <code> joins it, or {CLI} create <name> --as <your name> starts a new one.");
    };
    let network = if network.name.is_empty() { format!("{YOURS} (still joining)") } else { network.name.clone() };
    let ready = if state.device.ready { "" } else { ", catching up" };
    format!("You're {} in {network}, on {device}{ready}.", state.looking())
}

/// How the CLI asks this machine's node (`agent()`).
struct Http;

impl one::api::client::Via for Http {
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, wait: Option<std::time::Duration>) -> Result<one::api::client::Answer> {
        let mut req = ureq::http::Request::builder().method(method).uri(format!("http://{}{path}", api::addr()));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        let agent = agent();
        let req = agent.configure_request(req.body(body.unwrap_or_default())?).timeout_global(wait).build();
        let resp = agent.run(req).map_err(unreached)?;
        let trace = resp.headers().get(one::protocol::header::TRACE).and_then(|h| h.to_str().ok()).map(str::to_string);
        Ok(one::api::client::Answer { status: resp.status().as_u16(), trace, body: Box::new(resp.into_body().into_reader()) })
    }

    fn tries(&self) -> Option<Value> {
        client_device::take_tries()
    }
}

fn node() -> Client {
    Client::new(Http, false)
}

/// An HTTP client for this machine's node, as one of its own programs; or, where none runs and this computer is a
/// device of its user, for their nodes, over iroh (`client_device::route`).
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).middleware(client_device::route).build().into()
}

fn request(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
    // Not even reached: the nodes tried are all there is to say.
    let mut resp = resp.map_err(unreached)?;
    let status = resp.status().as_u16();
    one::api::client::answered(status, resp.body_mut().read_json()?)
}

fn unreached(err: ureq::Error) -> anyhow::Error {
    match client_device::active() {
        true => anyhow::anyhow!("{err}"),
        false if matches!(&err, ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::ConnectionRefused) => anyhow::anyhow!(client_device::not_here()),
        false => anyhow::Error::new(err).context(client_device::not_here()),
    }
}

