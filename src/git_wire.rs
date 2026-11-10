//! Telling git on this machine to reach the network's apps' git (`<address>/<app>/…`) through this program, as `git-remote-<CLI>`
//! (src/git_remote.rs), which a node does as it starts, and `one-dev throwaway git` does for its own network only.

use anyhow::{Context, Result, bail};

use crate::config::CLI;

/// The name git runs this program by for `<CLI>::` addresses.
pub fn helper() -> String {
    format!("git-remote-{CLI}")
}

/// What tells git `address`'s apps are reached as `<CLI>::…`: a config key and its value.
pub fn instead_of(address: &str) -> (String, String) {
    let address = address.trim_end_matches('/');
    (format!("url.{CLI}::{address}/.insteadOf"), format!("{address}/"))
}

/// What a node's git config (`<home>/git-config`, which the user's includes) says of its home: git runs the remote
/// helper with the user's environment, not the node's, so on a machine where `<CLI>_HOME` isn't the default the helper
/// is told here where its node is.
pub fn home_config(home: &std::path::Path) -> String {
    let quoted = home.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    format!("[{CLI}]\n\thome = \"{quoted}\"\n")
}

/// The home of the node that told git of it (`home_config`), if one did.
pub fn node_home() -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git").args(["config", "--get", &format!("{CLI}.home")]).output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').into()).filter(|p: &std::path::PathBuf| !p.as_os_str().is_empty())
}

pub fn link_in(dir: &std::path::Path) -> Result<()> {
    let me = std::env::current_exe()?.canonicalize()?;
    let link = dir.join(format!("{}{}", helper(), std::env::consts::EXE_SUFFIX));
    let _ = std::fs::remove_file(&link);
    crate::os::link_program(&me, &link).with_context(|| format!("git reaches your repos through {}, which couldn't be made: ln -s {} {}", link.display(), me.display(), link.display()))
}

/// `wire`, in the background, for a device that's this computer's own: a throwaway network leaves its git as it is.
pub fn wire_here(address: String) {
    if matches!(crate::mode::here(), crate::mode::Mode::Own) {
        std::thread::spawn(move || {
            if let Err(err) = wire(&address) {
                tracing::warn!("{err:#}");
            }
        });
    }
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
        // Beside the `<CLI>` users run (a link to it, on a Mac), else beside this program.
        let dir = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).find(|d| d.join(format!("{CLI}{}", std::env::consts::EXE_SUFFIX)).is_file()).unwrap_or_default())
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

fn git(args: &[&str]) -> bool {
    std::process::Command::new("git").args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)).is_file()))
}
