pub mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
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

fn user_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| ".".into())
}
pub mod entry;
pub mod git;
pub mod members;
pub mod store;
pub mod sync;
pub mod agents;
pub mod api;
pub mod apps;
pub mod builtin;
pub mod bundle;
pub mod deploy;
pub mod models;
pub mod load;
pub mod logos;
pub mod native;
pub mod node;
pub mod outbound;
pub mod providers;
pub mod release;
pub mod schedules;
pub mod vault;
pub mod shell;
