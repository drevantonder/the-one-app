//! The One App's program, every machine's: the node, oned, the CLI and MCP, and what the Cloudflare node and clients
//! share with them. The domain's words are GLOSSARY.md's; each module's doc says what it is.

pub mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));

    /// How this build's people sign in (`sign_in` in the config): with a passkey, or with their workplace's Google account.
    #[derive(PartialEq, Eq)]
    pub enum SignIn {
        Passkey,
        Google,
    }
}

use std::path::PathBuf;

/// Where this machine's node keeps its data: `<CLI>_HOME`, else `~/.local/share/<cli>`.
pub fn home() -> PathBuf {
    match std::env::var_os(format!("{}_HOME", config::CLI.to_uppercase())) {
        Some(dir) => dir.into(),
        None => std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| user_home().join(".local/share")).join(config::CLI),
    }
}

/// Where it keeps what it can download or build again: inside a home given with `<CLI>_HOME`, so a node given one
/// writes nowhere else; otherwise `~/.cache/<cli>`.
pub fn cache() -> PathBuf {
    match std::env::var_os(format!("{}_HOME", config::CLI.to_uppercase())) {
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
#[cfg(feature = "node")]
pub mod backups;
pub mod bucket;
pub mod call;
pub mod database;
pub mod sign_in_proof;
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
#[cfg(feature = "node")]
pub mod core_tools;
pub mod app;
#[cfg(feature = "node")]
pub mod git_wire;
#[cfg(feature = "node")]
pub mod handoff;
#[cfg(feature = "node")]
pub mod load;
#[cfg(feature = "node")]
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
pub mod schedules;
#[cfg(feature = "node")]
pub mod services;
#[cfg(feature = "node")]
pub mod mode;
#[cfg(feature = "node")]
pub mod oned;
pub mod cloudflare;
pub mod keys;
pub mod protocol;
pub mod push;
pub mod screen;
pub mod web;
pub mod state;
pub mod settings_tools;
pub mod vault;
