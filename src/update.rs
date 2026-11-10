//! `one update`: this device taking the newest signed build by itself, with a GET of the record at its network's address
//! and nothing else of the network: no background program, relay, or who's who, so a device stuck on anything else can
//! still take the fix. It imports only `latest` and `release` of what updating is (a test holds to that), then makes oned
//! exit, for its supervisor to bring it back on the new build.

use std::path::Path;

use anyhow::{Context, Result, ensure};

use crate::config::{BUILD, CLI, COMMITTED, TARGET};
use crate::latest::{self, Latest, Thing};
use crate::release;

pub mod cli;

/// The file in a device's home holding its network's address, the only thing an update needs that's the network's.
/// The address itself is a setting in who's who, which this can't read: the node keeps it here, each time it checks.
const ADDRESS: &str = "address";
/// The file in a device's home holding the hash of the build it last put in place, and hasn't restarted onto yet.
const TAKEN: &str = "release";
/// The file in a device's home holding the pid of its running oned.
pub const PID: &str = "pid";

/// Notes the network's address where a device can read it with nothing running: until then `one update` can't start.
pub fn keep_address(home: &Path, address: &str) -> std::io::Result<()> {
    match std::fs::read_to_string(home.join(ADDRESS)) {
        Ok(kept) if kept == address => Ok(()),
        _ => std::fs::write(home.join(ADDRESS), address),
    }
}

/// The hash of the build last put in place here, as `latest::wanted` takes it.
pub fn taken(home: &Path) -> String {
    std::fs::read_to_string(home.join(TAKEN)).unwrap_or_default().trim().to_string()
}

/// Notes that the build with this hash was put in place here.
pub fn took(home: &Path, sha256: &str) -> std::io::Result<()> {
    std::fs::write(home.join(TAKEN), sha256)
}

/// Checks the record and, if it names a newer build for this computer, replaces this program with it and has oned exit
/// so it comes back on it. Anything wrong (a bad signature, an older build, the wrong target, a failed download) leaves
/// the program as it was. → what happened, to say.
pub fn run() -> Result<String> {
    let home = crate::home();
    let key = crate::mode::here().release_key();
    ensure!(!key.is_empty(), "this custom build has no release key, so nothing can be released to it");
    ensure!(!release::in_app_bundle(), "this device's program is inside its app, which updates as a whole: {CLI} update can't replace it");
    let address = std::fs::read_to_string(home.join(ADDRESS)).ok().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).context("this device doesn't know its network's address yet: its background program notes it once it's joined and running")?;
    let Some(text) = latest::fetch(&address)? else { return Ok(format!("Nothing has been published at {address} yet.")) };
    let record = Latest::read(key, &text).with_context(|| format!("the record at {address}"))?;
    let thing = record.get(&Thing::node_name(TARGET)).with_context(|| format!("the record at {address} has no build for {TARGET}"))?;
    if !latest::wanted(thing, BUILD, COMMITTED, "") {
        return Ok(format!("This device is current, on {BUILD}. The newest published is {}.", thing.commit));
    }
    let bytes = latest::download(&address, thing)?;
    release::install_commit(&thing.commit, &bytes)?;
    if let Err(err) = took(&home, &thing.sha256) {
        eprintln!("{CLI}: couldn't note the update ({err}), so this device may fetch it again");
    }
    Ok(if restart_oned(&home) { format!("Updated this device to {}, and restarting it.", thing.commit) } else { format!("Updated this device to {}. Nothing was running here to restart.", thing.commit) })
}

/// Asks the oned this home's pid file names to exit, if it's still running: Replace first, then this, so being killed
/// partway leaves a whole program, whichever it is. → whether one was running.
fn restart_oned(home: &Path) -> bool {
    let Some(pid) = std::fs::read_to_string(home.join(PID)).ok().and_then(|p| p.trim().parse::<u32>().ok()) else { return false };
    // Only a process of this program's: a pid left from a stopped one may be anyone's by now.
    if !crate::os::is_program(pid, CLI) && !crate::os::is_program(pid, &crate::oned::name()) {
        return false;
    }
    crate::os::ask_pid_to_stop(pid as i32);
    true
}
