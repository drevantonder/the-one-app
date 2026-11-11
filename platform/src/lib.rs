//! What differs on Mac, Windows and Linux: where a device keeps its files (`home`, `cache`), how its programs start and
//! stop each other (`os`), small files written whole (`files`), asking URLs (`hedge`, `reach`), and whether this process is the user's
//! own network's or a throwaway's (`mode`).

#[cfg(feature = "machine")]
pub mod os;
pub mod files;
pub mod hedge;
#[cfg(feature = "machine")]
pub mod reach;
#[cfg(feature = "machine")]
pub mod mode;

use custom_build::config;
use std::path::PathBuf;

/// The path this program runs from, to start it again: its file, where a rebuild or update has replaced it (Linux then names
/// the replaced file "… (deleted)", which starts nothing).
pub fn this_program() -> std::io::Result<PathBuf> {
    Ok(PathBuf::from(std::env::current_exe()?.to_string_lossy().trim_end_matches(" (deleted)")))
}

/// Where this machine's node keeps its data: `<CLI>_HOME`, else `~/.local/share/<cli>`, on Windows the user's own
/// `%LOCALAPPDATA%\<cli>`.
pub fn home() -> PathBuf {
    match std::env::var_os(config::env("HOME")) {
        Some(dir) => dir.into(),
        None if cfg!(windows) => windows_data().join(config::CLI),
        None => std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| user_home().join(".local/share")).join(config::CLI),
    }
}

fn windows_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| user_home().join("AppData/Local"))
}

/// Where it keeps what it can download or build again: inside a home given with `<CLI>_HOME`, so a node given one
/// writes nowhere else; otherwise `~/.cache/<cli>`, on Windows beside its data.
pub fn cache() -> PathBuf {
    match std::env::var_os(config::env("HOME")) {
        Some(dir) => PathBuf::from(dir).join("cache"),
        None if cfg!(windows) => home().join("cache"),
        None => std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| user_home().join(".cache")).join(config::CLI),
    }
}

pub fn user_home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| ".".into())
}
