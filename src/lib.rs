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

pub(crate) fn user_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| ".".into())
}
// What the Cloudflare node (cloudflare/node/) and the native apps' client (client/) share with nodes on machines; the rest
// is only theirs (the `node` feature).
#[cfg(feature = "node")]
pub mod backups;
pub mod bucket;
pub mod call;
#[cfg(feature = "db")]
pub mod database;
#[cfg(feature = "db")]
pub mod db;
pub mod entry;
#[cfg(feature = "node")]
pub mod sign_in;
pub mod sign_in_proof;
#[cfg(feature = "node")]
pub mod skills;
pub mod labs;
pub mod members;
#[cfg(feature = "db")]
pub mod permissions;
pub mod rank;
#[cfg(feature = "node")]
pub mod store;
#[cfg(feature = "node")]
pub mod replica;
#[cfg(feature = "node")]
pub mod harnesses;
#[cfg(feature = "node")]
pub mod api;
#[cfg(feature = "node")]
pub mod apps;
#[cfg(feature = "node")]
pub mod handoff;
pub mod bundle;
#[cfg(feature = "node")]
pub mod deploy;
#[cfg(feature = "node")]
pub mod load;
#[cfg(feature = "node")]
pub mod logos;
#[cfg(feature = "node")]
pub mod node;
#[cfg(feature = "node")]
pub mod outbound;
#[cfg(feature = "node")]
pub mod release;
#[cfg(feature = "node")]
pub mod schedules;
#[cfg(feature = "node")]
pub mod services;
#[cfg(feature = "node")]
pub mod oned;
#[cfg(feature = "node")]
pub mod service;
#[cfg(feature = "node")]
pub mod sandbox;
pub mod cloudflare;
pub mod keys;
pub mod push;
#[cfg(feature = "db")]
pub mod log;
pub mod screen;
#[cfg(feature = "db")]
pub mod state;
#[cfg(feature = "db")]
pub mod vault;
