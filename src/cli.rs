//! The command line's built-in commands, each defined by its feature (`Command`), and what they share: how one fails
//! (`Failed`, its exit code), asking the person before what can't be undone, and the help made from them.

use std::io::IsTerminal;

use anyhow::Result;
use serde_json::{Map, Value};

use crate::config::CLI;
use crate::api::Client;

/// Why a command failed, when its exit code says more than 1: what was typed is wrong (2), or it was refused (3).
#[derive(Debug)]
pub enum Failed {
    Typed(String),
    Refused(String),
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failed::Typed(m) | Failed::Refused(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Failed {}

/// What was typed, for a command to run: the words (its own first), the flags, and the node to ask.
pub struct Given<'a> {
    pub words: &'a [String],
    pub flags: &'a Map<String, Value>,
    /// Everything typed, as typed.
    pub args: &'a [String],
    pub node: &'a Client,
    /// The agent running it, if one is.
    pub agent: Option<&'a str>,
    /// What a tool gives back is printed as JSON (`--json`).
    pub json: bool,
    /// A call prints what it did, and where, after its answer (`--trace`).
    pub trace: bool,
    /// The command line's own words, for what was mistyped.
    pub commands: &'a [String],
}

impl Given<'_> {
    /// The words, to match on.
    pub fn words(&self) -> Vec<&str> {
        self.words.iter().map(String::as_str).collect()
    }

    pub fn flag(&self, name: &str) -> Option<&str> {
        self.flags.get(name).and_then(Value::as_str)
    }

    pub fn has(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    /// Refuses a flag the command doesn't take, rather than running as if it weren't given.
    pub fn takes_only(&self, takes: &[&str]) -> Result<()> {
        match self.flags.keys().find(|k| !takes.contains(&k.as_str())) {
            Some(other) => Err(Failed::Typed(format!("--{other} isn't an input of this: see --help")).into()),
            None => Ok(()),
        }
    }
}

/// Some of the built-in commands, as a feature defines them: what `help all` says of each, and what runs them.
pub struct Command {
    /// Each form, as typed after the CLI's name (`vault set <item> <field> [--secret]`), with what it does.
    pub help: Vec<(String, String)>,
    /// Runs what was typed, if it's one of these commands: false if it isn't.
    pub run: fn(&Given) -> Result<bool>,
}

/// Where what a command does starts on its line in `help all`.
const ABOUT_AT: usize = 39;
/// How wide `help all` is.
const WIDTH: usize = 118;

/// The commands' help, every form of each as `help all` lists them.
pub fn help(commands: &[Command]) -> String {
    let mut out = String::new();
    for (usage, about) in commands.iter().flat_map(|c| &c.help) {
        let mut line = format!("  {CLI} {usage}");
        if line.chars().count() + 1 >= ABOUT_AT {
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        let mut words = about.split(' ').filter(|w| !w.is_empty()).peekable();
        while words.peek().is_some() {
            let pad = ABOUT_AT - line.chars().count();
            line.push_str(&" ".repeat(pad));
            let mut first = true;
            while let Some(word) = words.peek() {
                if !first && line.chars().count() + 1 + word.chars().count() > WIDTH {
                    break;
                }
                if !first {
                    line.push(' ');
                }
                line.push_str(word);
                words.next();
                first = false;
            }
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// Goes ahead with `command`, which sends or can't be undone, given --confirm; else asks at a terminal, and elsewhere
/// (an agent, or piped) refuses.
pub fn agreed(confirmed: bool, command: &str) -> Result<()> {
    if confirmed || asked_yes(command)? {
        return Ok(());
    }
    Err(Failed::Refused(format!("{command} sends or can't be undone, and the person hasn't agreed to it: ask them, and give --confirm only if they say yes to this very action")).into())
}

/// Asks the person at a terminal whether to go ahead with `command`: false with no one there to ask, and refused if they
/// say no.
pub fn asked_yes(command: &str) -> Result<bool> {
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Ok(false);
    }
    eprint!("{command} sends or can't be undone. Go ahead? [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
        return Err(Failed::Refused("stopped; nothing changed".into()).into());
    }
    Ok(true)
}
