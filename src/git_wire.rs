//! Telling git on this machine to reach the network's repos (`<address>/repos/…`) through this program, as `git-remote-<CLI>`
//! (src/git_remote.rs), which a node does as it starts, `sign-in` does, and `one throwaway git` does for its own network only.

use anyhow::{Context, Result, bail};

use crate::config::CLI;

/// The name git runs this program by for `<CLI>::` addresses.
pub fn helper() -> String {
    format!("git-remote-{CLI}")
}

/// What tells git `address`'s repos are reached as `<CLI>::…`: a config key and its value.
pub fn instead_of(address: &str) -> (String, String) {
    let address = address.trim_end_matches('/');
    (format!("url.{CLI}::{address}/repos/.insteadOf"), format!("{address}/repos/"))
}

pub fn link_in(dir: &std::path::Path) -> Result<()> {
    let me = std::env::current_exe()?.canonicalize()?;
    let link = dir.join(helper());
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&me, &link).with_context(|| format!("git reaches your repos through {}, which couldn't be made: ln -s {} {}", link.display(), me.display(), link.display()))
}

/// Tells git on this machine to reach `address`'s repos through this program, and makes sure git finds it as
/// `git-remote-<CLI>`, linked beside it.
pub fn wire(address: &str) -> Result<()> {
    let address = address.trim_end_matches('/');
    // A Mac's git, before its developer tools are installed, only offers to install them.
    let tools = !cfg!(target_os = "macos") || std::process::Command::new("xcode-select").arg("-p").output().is_ok_and(|o| o.status.success());
    if !tools || !git(&["--version"]) {
        return Ok(());
    }
    if !on_path(&helper()) {
        // Beside the `<CLI>` people run (a link to it, on a Mac), else beside this program.
        let dir = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).find(|d| d.join(CLI).is_file()).unwrap_or_default())
            .filter(|d| !d.as_os_str().is_empty())
            .or_else(|| std::env::current_exe().ok()?.canonicalize().ok()?.parent().map(|p| p.to_path_buf()))
            .context("this program is in no folder")?;
        link_in(&dir)?;
    }
    let (key, value) = instead_of(address);
    if !git(&["config", "--global", "--replace-all", &key, &value]) {
        bail!("git couldn't be told where your repos are: git config --global {key} {value}");
    }
    Ok(())
}

pub fn unwire(address: &str) {
    git(&["config", "--global", "--unset-all", &instead_of(address).0]);
}

fn git(args: &[&str]) -> bool {
    std::process::Command::new("git").args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}
