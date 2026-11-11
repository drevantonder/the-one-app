//! `one-dev`, for whoever builds this repo: sending out its builds (`ship`), and a throwaway network of their
//! checkout's build (`throwaway`). It asks this computer's app runtime as one of its programs, and runs the `one` program Cargo
//! builds beside it for what that does: an app runtime, the Workers, and any command in a throwaway network.

use std::path::PathBuf;

use anyhow::Result;
use api::Client;
use ::custom_build::config::{CLI, NAME};

mod build;
mod ship;
mod symbols;
mod throwaway;

/// What this program is called, beside the custom build's own: `one-dev` beside `one`.
pub fn dev() -> String {
    format!("{CLI}-dev")
}

/// The program beside this one that is the app runtime, the Workers and every command of a network: the custom build's own
/// where it's installed (`ptc` beside `ptc-dev`), else the `one` Cargo builds beside it in a checkout (cli/).
pub fn program() -> Result<PathBuf> {
    let me = platform::this_program()?;
    let installed = me.with_file_name(CLI);
    Ok(if installed.is_file() { installed } else { me.with_file_name("one") })
}

/// This computer's onet, asked as one of its programs.
pub fn loopback() -> Client {
    throwaway::loopback_at(&platform::home(), &api::local::addr())
}

/// An HTTP client for this machine's programs.
pub fn agent() -> ureq::Agent {
    api::local::agent()
}

fn commands() -> Vec<api::cli::Command> {
    vec![ship::commands(), symbols::command(), throwaway::command()]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = run(args) {
        eprintln!("{}: {err:#}", dev());
        std::process::exit(api::cli::exit_code(&err));
    }
}

fn run(args: Vec<String>) -> Result<()> {
    // A throwaway network's command is the `one` program's, run in it: what follows is read there.
    if args.first().map(String::as_str) == Some("throwaway") {
        return throwaway::run(&args[1..]);
    }
    let (positional, flags) = api::tools::split(&args);
    let api = loopback();
    let forms: Vec<(String, String)> = commands().into_iter().flat_map(|c| c.help).collect();
    let given = api::cli::Given { words: &positional, flags: &flags, args: &args, api: &api, agent: None, json: false, trace: false, commands: &forms };
    let asked_help = positional.is_empty() || positional[0] == "help" || flags.contains_key("help") || flags.contains_key("h");
    if !asked_help {
        for command in commands() {
            if (command.run)(&given)? {
                return Ok(());
            }
        }
    }
    let first = positional.first().filter(|w| *w != "help");
    let these: Vec<&(String, String)> = forms.iter().filter(|(usage, _)| first.is_some_and(|w| usage.split_whitespace().next() == Some(w))).collect();
    if first.is_some() && these.is_empty() {
        return Err(api::cli::Failed::Typed(format!("{} isn't a command of {}: {} help lists them", positional[0], dev(), dev())).into());
    }
    if !these.is_empty() {
        print!("Usage:\n{}", api::cli::help_as(&dev(), these));
        return Ok(());
    }
    println!(
        "{}: for whoever builds {NAME} from its repo. Users of a network, and their agents, use {CLI}.\n\nUsage:\n{}",
        dev(),
        api::cli::help_as(&dev(), &forms)
    );
    Ok(())
}
