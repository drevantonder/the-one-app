//! A CLI that's only a device of the throwaway network's user, as on a computer where no node runs.

use std::path::Path;

use anyhow::{Context, Result};
use one::config::CLI;

use super::super::devices::{approve_code, is_code};

/// `command`, run by this build's CLI as on a computer where no node runs: a client device of the throwaway network's
/// user, with a home of its own beside it (`cli/`), which asks their nodes over iroh as every client does, in run
/// priority's order (`client_device`). The first time it signs in with no passkey, and the throwaway network's machine
/// approves the code it shows.
pub(crate) fn cli(dir: &Path, settings: &one::cloudflare::Settings, command: &[String]) -> Result<()> {
    let program = super::super::network::program(dir)?;
    let home = dir.join("cli");
    std::fs::create_dir_all(&home)?;
    // No node is taken as here, nor anything of this machine's in its home, git's own config included (sign-in tells it where repos are).
    let as_device = |args: &[String]| {
        let mut c = std::process::Command::new(&program);
        c.args(args).env(one::config::env("NO_NODE"), "1").env(one::config::env("HOME"), &home).env("XDG_CONFIG_HOME", &home).env("GIT_CONFIG_GLOBAL", home.join("gitconfig")).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
        c
    };
    let joined = home.join("joined");
    if !joined.exists() {
        use std::io::BufRead;
        eprintln!("Signing in a CLI as a device of the throwaway network's user…");
        let mut child = as_device(&["sign-in".into(), "--address".into(), settings.address.clone()]).stdout(std::process::Stdio::piped()).spawn()?;
        for line in std::io::BufReader::new(child.stdout.take().context("no output from signing in")?).lines() {
            let line = line?;
            let said = line.trim();
            eprintln!("{line}");
            if said.starts_with(&settings.address) && said.contains("/sign-in") {
                crate::throwaway::sign_in::approve_on_throwaway(&settings.address, said, None)?;
            } else if is_code(said) {
                approve_code(dir, said)?;
            }
        }
        anyhow::ensure!(child.wait()?.success(), "the CLI didn't sign in as a device");
        std::fs::write(&joined, "")?;
    }
    if command.is_empty() {
        println!("A CLI that's only a device of the throwaway network's user is signed in: {CLI}-dev throwaway client cli <command> runs one as it.");
        return Ok(());
    }
    let status = as_device(command).status()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
