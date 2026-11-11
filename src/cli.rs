//! The command line's built-in commands, each defined by its feature (`Command`), and what they share: how one fails
//! (`Failed`, its exit code), asking the user before what sends or can't be undone, and the help made from them.

use std::io::IsTerminal;

use anyhow::Result;

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
    pub flags: &'a crate::tools::Flags,
    /// Everything typed, as typed.
    pub args: &'a [String],
    pub node: &'a Client,
    /// The agent running it, if one is.
    pub agent: Option<&'a str>,
    /// What a tool gives back is printed as JSON (`--json`).
    pub json: bool,
    /// A call prints what it did, and where, after its answer (`--trace`).
    pub trace: bool,
    /// The command line's own commands (`Command.help`), for what was mistyped.
    pub commands: &'a [(String, String)],
}

impl Given<'_> {
    pub fn words(&self) -> Vec<&str> {
        self.words.iter().map(String::as_str).collect()
    }

    pub fn flag(&self, name: &str) -> Option<&str> {
        self.flags.get(name)?.last()?.as_deref()
    }

    /// Each value of a flag given more than once (`--device a --device b`).
    pub fn values(&self, name: &str) -> Vec<&str> {
        self.flags.get(name).into_iter().flatten().filter_map(Option::as_deref).collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    /// Refuses a flag the command doesn't take, rather than running as if it weren't given.
    pub fn takes_only(&self, takes: &[&str]) -> Result<()> {
        match self.flags.keys().find(|k| !takes.contains(&k.as_str())) {
            Some(other) => Err(Failed::Typed(format!("--{other} isn't an input of this (see --help)")).into()),
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

/// The words that begin commands' forms: what the command line takes before any app's name.
pub fn words(forms: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = forms.iter().filter_map(|(usage, _)| usage.split_whitespace().next()).filter(|w| w.chars().all(|c| c.is_ascii_lowercase() || c == '-')).map(String::from).collect();
    out.dedup();
    out
}

/// Commands' forms (`Command.help`) as `help all` lays them out.
pub fn help<'a>(forms: impl IntoIterator<Item = &'a (String, String)>) -> String {
    help_as(CLI, forms)
}

/// `help`, for another program than the CLI: `one-dev`'s.
pub fn help_as<'a>(program: &str, forms: impl IntoIterator<Item = &'a (String, String)>) -> String {
    let mut out = String::new();
    for (usage, about) in forms {
        let mut line = format!("  {program} {usage}");
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

/// Asks the user at a terminal whether to go ahead with `command`: false with no one there to ask, and refused if they
/// say no.
pub fn asked_yes(command: &str) -> Result<bool> {
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Ok(false);
    }
    eprint!("{command} {}. Go ahead? [y/N] ", crate::core_tools::SENDS);
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
        return Err(Failed::Refused("stopped; nothing changed".into()).into());
    }
    Ok(true)
}

/// How long ago `ms` (since 1970) was, for users: "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(ms: u64) -> String {
    let s = crate::node::now_ms().saturating_sub(ms) / 1000;
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        86400..172800 => "1 day ago".into(),
        _ => format!("{} days ago", s / 86400),
    }
}

/// Whether a device is on, for users: "online" while its socket is open at the relay, else when it was last seen
/// ("seen 5 min ago"), and nothing for one never seen.
pub fn presence(online: bool, seen: Option<u64>) -> String {
    match (online, seen) {
        (true, _) => "online".into(),
        (false, Some(at)) => format!("seen {}", ago(at)),
        (false, None) => String::new(),
    }
}

/// Why a device that doesn't answer is off, for users: "mac was last seen 5 min ago", "mac is off" for one never seen.
pub fn off(name: &str, seen: Option<u64>) -> String {
    match seen {
        Some(at) => format!("{name} was last seen {}", ago(at)),
        None => format!("{name} is off"),
    }
}

/// What `<cli> cloudflare deploy` and the shell `update` runs it in exit with, which `update` acts on.
pub mod exit {
    /// Refused: a command that wasn't allowed, or needs `--confirm` (`tools::exit_code`).
    pub const REFUSED: i32 = 3;
    /// The device's node isn't the build being sent out yet: ask again once it is (the exit code `sh` programs use for
    /// "try again", EX_TEMPFAIL).
    pub const NOT_YET: i32 = 75;
}
