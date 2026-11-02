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
// What the serverless node (serverless/) shares with nodes on machines; the rest is only theirs.
#[cfg(not(target_family = "wasm"))]
pub mod backups;
pub mod bucket;
pub mod call;
pub mod db;
pub mod entry;
#[cfg(not(target_family = "wasm"))]
pub mod sign_in;
#[cfg(not(target_family = "wasm"))]
pub mod skills;
pub mod members;
pub mod rank;
#[cfg(not(target_family = "wasm"))]
pub mod store;
#[cfg(not(target_family = "wasm"))]
pub mod replica;
#[cfg(not(target_family = "wasm"))]
pub mod agents;
#[cfg(not(target_family = "wasm"))]
pub mod api;
#[cfg(not(target_family = "wasm"))]
pub mod apps;
pub mod bundle;
#[cfg(not(target_family = "wasm"))]
pub mod deploy;
#[cfg(not(target_family = "wasm"))]
pub mod messenger;
#[cfg(not(target_family = "wasm"))]
pub mod models;
#[cfg(not(target_family = "wasm"))]
pub mod load;
#[cfg(not(target_family = "wasm"))]
pub mod logos;
#[cfg(not(target_family = "wasm"))]
pub mod node;
#[cfg(not(target_family = "wasm"))]
pub mod outbound;
#[cfg(not(target_family = "wasm"))]
pub mod push;
#[cfg(not(target_family = "wasm"))]
pub mod release;
#[cfg(not(target_family = "wasm"))]
pub mod schedules;
#[cfg(not(target_family = "wasm"))]
pub mod serverless;
#[cfg(not(target_family = "wasm"))]
pub mod sidecars;
pub mod kernel;
pub mod keys;
pub mod log;
pub mod screen;
pub mod state;
pub mod vault;
#[cfg(not(target_family = "wasm"))]
pub mod shell;
