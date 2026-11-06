
use anyhow::{Result, bail};
use one::config::{CLI, NAME, YOURS};
use one::cli::{Command, Given};
use one::api;
use serde_json::{Map, Value, json};

mod client_device;
mod git_remote;
mod network;
mod sign_in;
mod throwaway;
mod update;

use one::api::Client;
use one::{build, tools};

/// Every built-in command, each feature's, in the order they're tried: the first that takes what was typed runs it.
fn commands() -> Vec<Command> {
    use one::node::{apps, backups, blobs, groups, services, storage, vault};
    vec![
        network::commands(),
        groups::commands(),
        apps::cli::commands(),
        vault::cli::commands(),
        one::cloudflare::cli::commands(),
        storage::cli::commands(),
        backups::cli::commands(),
        services::cli::commands(),
        one::logs::commands(),
        update::commands(),
        blobs::cli::commands(),
        own(),
        one::skills::commands(),
        one::code::commands(),
        one::mcp::commands(),
        throwaway::command(),
        one::tools::commands(),
    ]
}

/// Every built-in command, with what it takes: `help all` prints it, and `<command> --help` its lines.
fn every_command() -> String {
    format!("Usage:\n{}\n{NAME} holds one network. For another context, run that context's own app.", one::cli::help(&commands()))
}

fn command_words() -> Vec<String> {
    let mut out: Vec<String> = commands().iter().flat_map(|c| &c.help).filter_map(|(usage, _)| usage.split_whitespace().next()).filter(|w| w.chars().all(|c| c.is_ascii_lowercase() || c == '-')).map(String::from).collect();
    out.dedup();
    out
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
        "{NAME}: your machines, and your team's, as one place for apps and agents.

  {CLI} search <words>        the tools for a task in every app, each with how to call it
  {CLI} <app>                 an app's tools and its guide
  {CLI} <app> <tool> --help     what a tool takes
  {CLI} <app> <tool> [words] [--input value]
                             run one, like {CLI} repos list
  {CLI} run <file>            a script that calls several tools; {CLI} search gives each call
  {CLI} skills                the guides agents load
  {CLI} status                who you are, and your devices
  {CLI} mcp [--code]          the local MCP server, for agents
{yours}
A tool's words fill its main inputs in order, a list repeats its flag, and inputs not given are read as JSON
from stdin. --file <input>=<path> gives a file, --out <path> saves what comes back, --device <name> runs it there,
and --confirm goes ahead with what sends or can't be undone, once the person agreed. --json prints what comes back
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
    // A throwaway's command is run again as a program of its network, which reads what follows itself.
    if args.first().map(String::as_str) == Some("throwaway") {
        return throwaway::run(&args[1..]);
    }
    // --json is everyone's, wherever it's typed: what a tool gives back, as JSON.
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let json = args[..end].iter().any(|a| a == "--json");
    // --trace too: a call prints what it did, and where, after its answer.
    let trace = args[..end].iter().any(|a| a == "--trace");
    let args: Vec<String> = args.iter().enumerate().filter(|(i, a)| *i >= end || (*a != "--json" && *a != "--trace")).map(|(_, a)| a.clone()).collect();
    let (positional, flags) = parse(&args);
    if positional.first().map(String::as_str) != Some("mcp") {
        // Printing into `| head` ends quietly, as other commands do, instead of panicking.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    let (agent, node, words) = (one::mcp::agent_from_env(), node(), command_words());
    let given = Given { words: &positional, flags: &flags, args: &args, node: &node, agent: agent.as_deref(), json, trace, commands: &words };
    // --help shows help, whatever the command: never runs it. An app's tool's is the app's own.
    if flags.contains_key("help") || positional.iter().any(|p| p == "-h") {
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
                    None => bail!("{}", tools::unknown(g.node, rest[0], None, Some(g.commands))?),
                },
                ["version"] if g.json => println!("{}", one::release::Version::this().json()),
                ["version"] => println!("{} {} ({})", NAME, one::config::BUILD, one::config::TARGET),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// A built-in command's help, for --help after it: every_command()'s lines for it, or help() when none are. None for
/// an app's tool, and for an app that isn't a built-in command too, whose help is their own.
fn command_help(node: &Client, words: &[String]) -> Option<String> {
    let first = words.first().filter(|w| *w != "-h")?;
    let text = every_command();
    // The lines for the most words given that have any: vault set's, else every vault line.
    let lines = |prefix: &str| {
        let mut out = String::new();
        let mut taking = false;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix(&format!("  {CLI} {prefix}")) {
                taking = rest.is_empty() || rest.starts_with(' ');
            } else if !line.starts_with("        ") {
                taking = false;
            }
            if taking {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    };
    let commands: Vec<&String> = words.iter().take_while(|w| !w.starts_with('-')).collect();
    let (said, out) = (1..=commands.len()).rev().map(|n| (n, lines(&commands[..n].iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" ")))).find(|(_, o)| !o.is_empty()).unwrap_or_default();
    let app = one::core_tools::and_apps(node).unwrap_or_default().iter().any(|a| a["name"] == first.as_str());
    match (app, out.is_empty()) {
        (true, _) if words.len() > 1 && said < commands.len() => None,
        (true, true) => None,
        (false, true) => Some(format!("{}\n", help(node))),
        (_, false) => Some(format!("Usage:\n{out}")),
    }
}

/// Splits `--key value` flags from positional words. A flag with no value is true.
fn parse(args: &[String]) -> (Vec<String>, Map<String, Value>) {
    let (mut positional, mut flags) = (Vec::new(), Map::new());
    let mut i = 0;
    while i < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            match args.get(i + 1).filter(|v| !v.starts_with("--")) {
                Some(v) => {
                    let value = serde_json::from_str(v).ok().filter(|v: &Value| !v.is_string()).unwrap_or_else(|| json!(v));
                    flags.insert(key.to_string(), value);
                    i += 2;
                }
                None => {
                    flags.insert(key.to_string(), json!(true));
                    i += 1;
                }
            }
        } else {
            positional.push(args[i].clone());
            i += 1;
        }
    }
    (positional, flags)
}

fn connection(node: &Client) -> String {
    let state = match node.get("/v1/state") {
        Ok(state) => state,
        Err(err) if client_device::active() => return format!("This computer is one of your devices, but {err:#}."),
        Err(_) => return format!("{}.", client_device::not_here()),
    };
    if let Some(said) = client_device::said(&state) {
        return format!("Connected as {said}. {CLI} status shows your devices; {CLI} apps what you can run.");
    }
    let device = state["device"]["name"].as_str().unwrap_or("this device");
    let i = &state["network"];
    if i.is_null() {
        return format!("{device} isn't in {YOURS} yet: {CLI} join <code> joins it, or {CLI} create <name> --as <your name> starts a new one.");
    }
    let network = i["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{YOURS} (still joining)"));
    let me = state["user"]["name"].as_str().or(state["me"]["name"].as_str()).unwrap_or("");
    let ready = if state["device"]["ready"] == true { "" } else { ", catching up" };
    format!("Connected to {network} as {me}, on {device}{ready}. {CLI} status shows its devices; {CLI} apps what you can run.")
}

/// How the CLI asks this machine's node (`agent()`): as whom this machine is signed in as, when `signed_in`.
struct Http {
    signed_in: bool,
}

impl one::api::client::Via for Http {
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, wait: Option<std::time::Duration>) -> Result<one::api::client::Answer> {
        let mut req = ureq::http::Request::builder().method(method).uri(format!("http://{}{path}", api::addr()));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        if self.signed_in && let Some(token) = token() {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let agent = agent();
        let req = agent.configure_request(req.body(body.unwrap_or_default())?).timeout_global(wait).build();
        let resp = agent.run(req).map_err(unreached)?;
        let trace = resp.headers().get(one::node::trace::HEADER).and_then(|h| h.to_str().ok()).map(str::to_string);
        Ok(one::api::client::Answer { status: resp.status().as_u16(), trace, body: Box::new(resp.into_body().into_reader()) })
    }

    fn tries(&self) -> Option<Value> {
        client_device::take_tries()
    }
}

fn node() -> Client {
    Client::new(Http { signed_in: true }, false)
}

/// The access token this machine is signed in with, sent with what it asks its node: none as a device, which asks as its
/// own key, its sign-in only for asking to be let in.
fn token() -> Option<String> {
    if client_device::active() {
        return None;
    }
    crate::sign_in::token().unwrap_or_else(|err| {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| eprintln!("{err:#}"));
        None
    })
}

fn to_node<B>(r: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    match token() {
        Some(token) => r.header("authorization", format!("Bearer {token}")),
        None => r,
    }
}

/// An HTTP client for this machine's node, as one of its own programs; or, where none runs and this computer is a
/// device of its person, for their nodes, over iroh (`client_device::instead`).
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).middleware(client_device::instead).middleware(api::as_this_machine).build().into()
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
        false => anyhow::Error::new(err).context(client_device::not_here()),
    }
}

