//! `one-dev`, for whoever builds this repo: its updates and releases (`update`), and a throwaway network of their
//! checkout's build (`throwaway`). It asks this machine's node as one of its programs, and runs the `one` program Cargo
//! builds beside it for what that does: a node, the Workers, and any command in a throwaway network.

use std::path::PathBuf;

use anyhow::Result;
use one::api::Client;
use one::config::{CLI, NAME};
use one::tools;

mod throwaway;
mod update;

/// What this program is called, beside the custom build's own: `one-dev` beside `one`.
pub fn dev() -> String {
    format!("{CLI}-dev")
}

/// The program beside this one that is the node, the Workers and every command of a network: the custom build's own
/// where it's installed (`ptc` beside `ptc-dev`), else the `one` Cargo builds beside it in a checkout (cli/).
pub fn program() -> Result<PathBuf> {
    let me = one::this_program()?;
    let installed = me.with_file_name(CLI);
    Ok(if installed.is_file() { installed } else { me.with_file_name("one") })
}

/// This machine's node, asked as one of its programs.
pub fn node() -> Client {
    throwaway::node_at(&one::home(), &one::api::addr())
}

/// An HTTP client for this machine's programs.
pub fn agent() -> ureq::Agent {
    one::api::agent()
}

fn commands() -> Vec<one::cli::Command> {
    vec![update::commands(), throwaway::command()]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = run(args) {
        eprintln!("{}: {err:#}", dev());
        std::process::exit(tools::exit_code(&err));
    }
}

fn run(args: Vec<String>) -> Result<()> {
    // A throwaway network's command is the `one` program's, run in it: what follows is read there.
    if args.first().map(String::as_str) == Some("throwaway") {
        return throwaway::run(&args[1..]);
    }
    let (positional, flags) = tools::split(&args);
    let node = node();
    let words: Vec<String> = commands().iter().flat_map(|c| &c.help).filter_map(|(usage, _)| usage.split_whitespace().next().map(String::from)).collect();
    let given = one::cli::Given { words: &positional, flags: &flags, args: &args, node: &node, agent: None, json: false, trace: false, commands: &words };
    if !(positional.is_empty() || positional[0] == "help" || flags.contains_key("help") || flags.contains_key("h")) {
        for command in commands() {
            if (command.run)(&given)? {
                return Ok(());
            }
        }
        if !words.contains(&positional[0]) {
            return Err(one::cli::Failed::Typed(format!("{} isn't a command of {}: {} help lists them", positional[0], dev(), dev())).into());
        }
    }
    println!(
        "{}: for whoever builds {NAME} from its repo. The people a network serves, and their agents, use {CLI}.\n\nUsage:\n{}",
        dev(),
        one::cli::help_as(&dev(), commands().iter().flat_map(|c| &c.help))
    );
    Ok(())
}
