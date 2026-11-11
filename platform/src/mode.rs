//! What this process is for: the user's own network (`Own`), or a throwaway network's (`one-dev throwaway`), which has a home
//! and address of its own and must touch nothing of the real one's (`~/.local/bin`, the harnesses' configs, git's config).
//! `one-dev throwaway` tells every program it starts, by `Throwaway::tell`, and it's read here once, as the program starts: no code
//! asks the environment, settings or a flag whether this is one.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;


pub enum Mode {
    Own,
    Throwaway(Throwaway),
}

/// A throwaway network, by its folder: its home, its programs' logs and pids, its data.
pub struct Throwaway {
    home: PathBuf,
    release_key: String,
}

const UI: &str = "ui-folder";
/// The private half of a throwaway network's own release key, and beside it its public half.
const RELEASE_KEY: &str = "release-key";
const RELEASE_KEY_PUBLIC: &str = "release-key.pub";

fn var() -> String {
    custom_build::config::env("THROWAWAY")
}

fn release_key_var() -> String {
    custom_build::config::env("RELEASE_KEY")
}

pub fn here() -> &'static Mode {
    static HERE: OnceLock<Mode> = OnceLock::new();
    HERE.get_or_init(|| match std::env::var_os(var()) {
        Some(home) => Mode::Throwaway(Throwaway { home: home.into(), release_key: std::env::var(release_key_var()).unwrap_or_default() }),
        None => Mode::Own,
    })
}

impl Mode {
    /// Where the programs of services are put that an app runtime fetches: `~/.local/bin`, but a throwaway
    /// network's in its home, so what it puts there never replaces the real network's program.
    pub fn bin(&self) -> PathBuf {
        match self {
            Mode::Throwaway(t) => t.home.join("bin"),
            Mode::Own if cfg!(windows) => crate::home().join("bin"),
            Mode::Own => crate::user_home().join(".local/bin"),
        }
    }

    /// Where a Mac's window is installed: `~/Applications/<name>.app`, but a throwaway network's computer's in its home.
    pub fn mac_app(&self) -> PathBuf {
        let folder = match self {
            Mode::Throwaway(t) => t.home.join("Applications"),
            Mode::Own => crate::user_home().join("Applications"),
        };
        folder.join(format!("{}.app", custom_build::config::NAME))
    }

    /// The public half of the key what this device takes must be signed with: this build's, or a throwaway network's
    /// own, so what it publishes never needs the real one.
    pub fn release_key(&self) -> &str {
        match self {
            Mode::Own => custom_build::config::RELEASE_KEY,
            Mode::Throwaway(t) => &t.release_key,
        }
    }
}

impl Throwaway {
    /// Makes `command` a program of the throwaway network in `home`, whose app runtime is at `addr`: its home and address, which
    /// it asks its app runtime at as one of this machine's programs, and its mode.
    pub fn tell(command: &mut std::process::Command, home: &Path, addr: &str) {
        command.env(var(), home).env(custom_build::config::env("HOME"), home).env(custom_build::config::env("ADDR"), addr);
        if let Ok(key) = std::fs::read_to_string(home.join(RELEASE_KEY_PUBLIC)) {
            command.env(release_key_var(), key.trim());
        }
    }

    /// The private half of the throwaway network in `home`'s own release key, made the first time, which `tell` then tells
    /// its programs the public half of.
    pub fn own_release_key(home: &Path) -> Result<ssh_key::PrivateKey> {
        let path = home.join(RELEASE_KEY);
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Ok(ssh_key::PrivateKey::from_openssh(text)?);
        }
        let key = ssh_key::PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&rand::random()));
        std::fs::create_dir_all(home)?;
        std::io::Write::write_all(&mut crate::os::private_file().write(true).create(true).truncate(true).open(&path)?, key.to_openssh(ssh_key::LineEnding::LF)?.as_bytes())?;
        std::fs::write(home.join(RELEASE_KEY_PUBLIC), key.public_key().to_openssh()?)?;
        Ok(key)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The folder its pages and Workers are read from instead of what its build holds: a checkout's root.
    pub fn ui(&self) -> Option<PathBuf> {
        Self::ui_in(&self.home)
    }

    /// `ui`, for the throwaway network in `home`.
    pub fn ui_in(home: &Path) -> Option<PathBuf> {
        Some(std::fs::read_to_string(home.join(UI)).ok()?.trim().into())
    }

    /// Says which folder that is for the throwaway network in `home`, read by its programs.
    pub fn set_ui(home: &Path, folder: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(home)?;
        std::fs::write(home.join(UI), folder.to_string_lossy().as_bytes())
    }
}

/// Notes a program the throwaway network in `dir` started, in a process group of its own, for `programs` to find: a
/// client's browser is `client`, which starting the network again leaves running.
pub fn record(dir: &Path, pid: u32, client: bool) -> Result<()> {
    let mut pids = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
    pids += &format!("{pid} {}{}\n", started(pid as i32).unwrap_or_default(), if client { " client" } else { "" });
    std::fs::write(dir.join("pids"), pids)?;
    Ok(())
}

/// When a process started, where this machine says (Linux's /proc): in clock ticks since it booted.
fn started(pid: i32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // After the command's name, in parentheses, which may hold spaces: the 22nd field.
    stat.rsplit_once(')')?.1.split_whitespace().nth(19).map(String::from)
}

/// The throwaway network's programs that still run, by the process groups they lead (`record`): only those that started when
/// they did, where this machine says, as another process may have taken a pid since. With `clients` false, not the
/// clients' browsers.
pub fn programs(dir: &Path, clients: bool) -> Vec<i32> {
    let pids = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
    let ours = |pid: i32, at: &str| at.is_empty() || started(pid).as_deref() == Some(at);
    pids.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next().unwrap_or("").to_string(), fields.next() == Some("client")))
        })
        .filter(|(pid, at, client)| ours(*pid, at) && (clients || !client))
        .map(|(pid, _, _)| pid)
        .collect()
}

/// Stops what the throwaway network in `home` started besides its app runtime, which then stops itself: its Workers and browser.
pub fn stop_others(home: &Path) {
    let me = std::process::id() as i32;
    for pid in programs(home, true) {
        if pid != me {
            crate::os::ask_group_to_stop(-pid);
        }
    }
    let _ = std::fs::remove_file(home.join("pids"));
}
