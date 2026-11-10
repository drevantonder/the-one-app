//! The One App's program, every machine's: the node, oned, the CLI and MCP, and what the Cloudflare node and clients
//! share with them. The domain's words are GLOSSARY.md's; each module's doc says what it is.

pub mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));

    /// The environment variable this build's programs read for `suffix`: its CLI's name in capitals, then `_<suffix>`
    /// (`ONE_HOME`), with a hyphen in the name an underscore.
    pub fn env(suffix: &str) -> String {
        format!("{}_{suffix}", CLI.to_uppercase().replace('-', "_"))
    }
}

use std::path::PathBuf;

/// The path this program runs from, to start it again: its file, where a rebuild or update has replaced it (Linux then names
/// the replaced file "… (deleted)", which starts nothing).
pub fn this_program() -> std::io::Result<PathBuf> {
    Ok(PathBuf::from(std::env::current_exe()?.to_string_lossy().trim_end_matches(" (deleted)")))
}

/// Where this machine's node keeps its data: `<CLI>_HOME`, else `~/.local/share/<cli>`.
pub fn home() -> PathBuf {
    match std::env::var_os(config::env("HOME")) {
        Some(dir) => dir.into(),
        None => std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| user_home().join(".local/share")).join(config::CLI),
    }
}

/// Where it keeps what it can download or build again: inside a home given with `<CLI>_HOME`, so a node given one
/// writes nowhere else; otherwise `~/.cache/<cli>`.
pub fn cache() -> PathBuf {
    match std::env::var_os(config::env("HOME")) {
        Some(dir) => PathBuf::from(dir).join("cache"),
        None => std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| user_home().join(".cache")).join(config::CLI),
    }
}

/// `s` as a part of a URL: its unreserved characters as they are, every other byte percent-encoded.
pub fn url_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub(crate) fn user_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| ".".into())
}
// What the Cloudflare node (cloudflare/node/) and the native apps' client (client/) share with nodes on machines; the rest
// is only theirs (the `node` feature).
pub mod backups;
pub mod bucket;
pub mod call;
pub mod database;
pub mod onboarding;
#[cfg(feature = "node")]
pub mod skills;
pub mod files;
pub mod labs;
pub mod permissions;
pub mod rank;
#[cfg(feature = "node")]
pub mod replica;
#[cfg(feature = "node")]
pub mod harnesses;
#[cfg(feature = "node")]
pub mod api;
#[cfg(feature = "node")]
pub mod cli;
#[cfg(feature = "node")]
pub mod build;
#[cfg(feature = "node")]
pub mod code;
#[cfg(feature = "node")]
pub mod mcp;
#[cfg(feature = "node")]
pub mod tools;
pub mod core_tools;
pub mod devices;
pub mod groups;
pub mod widgets;
pub mod app;
#[cfg(feature = "node")]
pub mod git_wire;
#[cfg(feature = "node")]
pub mod handoff;
pub mod load;
pub mod insights;
pub mod logs;
#[cfg(feature = "node")]
pub mod logos;
#[cfg(feature = "node")]
pub mod reach;
#[cfg(feature = "node")]
pub mod node;
#[cfg(feature = "node")]
pub mod release;
#[cfg(feature = "node")]
pub mod latest;
#[cfg(feature = "node")]
pub mod schedules;
#[cfg(feature = "node")]
pub mod services;
#[cfg(feature = "node")]
pub mod mode;
#[cfg(feature = "node")]
pub mod oned;
pub mod cloudflare;
pub mod keys;
pub mod openapi;
pub mod protocol;
pub mod push;
pub mod screen;
pub mod ui_files;
pub mod ui_folder;
pub mod web;
pub mod state;
pub mod settings;
pub mod vault;
