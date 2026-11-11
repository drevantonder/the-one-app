//! `one update`: this device taking the newest signed build, with a GET of the record at its network's address and
//! nothing else of the network: no onet, app runtime, relay, or who's who, so a device stuck on anything else can still
//! take the fix. It's the only way a device changes build, run by a person or an agent, or by Update in Settings ›
//! Devices, which asks the device's onet to run it. It imports only `latest` and `release` of what updating is (a test
//! holds to that). It then makes oned exit, for its supervisor to bring it back on the new build. A Mac's `one update`
//! also takes its window, the app the record names beside the program (`mac`).

use std::path::Path;

use anyhow::{Context, Result, ensure};

use crate::config::{BUILD, CLI, COMMITTED, TARGET};
use crate::latest::{self, Latest, Thing};
use crate::release;

pub mod cli;
mod mac;

/// The file in a device's home holding its network's address, the only thing an update needs that's the network's.
/// The address itself is a setting in who's who, which this can't read: the node keeps it here, each time it checks.
const ADDRESS: &str = "address";
/// The file in a device's home holding the pid of its running oned.
pub const PID: &str = "pid";
/// The file in a device's home holding the build its running oned began on: its program's file may be a newer one by now.
pub const RUNNING: &str = "running";
/// The folder in a device's home an update works in, and the file there saying why the last one stopped, if it did.
const FOLDER: &str = "update";
const WHY: &str = "why";

/// Why this device couldn't take the newest build last time, if it couldn't: what `one update` said, for `status` to show.
pub fn why(home: &Path) -> Option<String> {
    std::fs::read_to_string(home.join(FOLDER).join(WHY)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Notes why this device couldn't take the newest build, or with none, that nothing stopped it.
fn say_why(home: &Path, why: Option<&str>) -> std::io::Result<()> {
    let file = home.join(FOLDER).join(WHY);
    match why {
        Some(why) => {
            std::fs::create_dir_all(home.join(FOLDER))?;
            std::fs::write(file, why)
        }
        None => match std::fs::remove_file(file) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        },
    }
}

/// Notes that the oned running here began on this build.
pub fn note_running(home: &Path) -> std::io::Result<()> {
    std::fs::write(home.join(RUNNING), BUILD)
}

/// Notes the network's address where a device can read it with nothing running: until then `one update` can't start.
pub fn keep_address(home: &Path, address: &str) -> std::io::Result<()> {
    match std::fs::read_to_string(home.join(ADDRESS)) {
        Ok(kept) if kept == address => Ok(()),
        _ => std::fs::write(home.join(ADDRESS), address),
    }
}

/// The network's address, as kept here (`keep_address`).
pub fn address(home: &Path) -> Option<String> {
    std::fs::read_to_string(home.join(ADDRESS)).ok().map(|a| a.trim().to_string()).filter(|a| !a.is_empty())
}

/// Checks the record and, if it names a newer build for this computer, replaces this program with it and has oned exit
/// so it comes back on it; on a Mac, takes the window it names too. Anything wrong (a bad signature, an older build, the
/// wrong target, a failed download) leaves what it's about as it was, and is kept for `why`. → what happened, to say.
pub fn run() -> Result<String> {
    let home = crate::home();
    let done = take(&home);
    if let Err(err) = say_why(&home, done.as_ref().err().map(|e| format!("{e:#}")).as_deref()) {
        eprintln!("{CLI}: couldn't note why this device didn't update: {err}");
    }
    done
}

fn take(home: &Path) -> Result<String> {
    let home = home.to_path_buf();
    let key = crate::mode::here().release_key();
    ensure!(!key.is_empty(), "this custom build has no release key, so nothing can be released to it");
    let address = address(&home).context("this device doesn't know its network's address yet: its background program notes it once it's joined and running")?;
    let Some(text) = latest::fetch(&address)? else { return Ok(format!("Nothing has been published at {address} yet.")) };
    let record = Latest::read(key, &text).with_context(|| format!("the record at {address}"))?;
    let thing = record.to_update_to(TARGET).with_context(|| format!("the record at {address} has no build for {TARGET}"))?;
    let said = if latest::wanted(thing, BUILD, COMMITTED) {
        let bytes = latest::download(&address, thing)?;
        release::install_commit(&thing.commit, &bytes)?;
        Some(if restart_oned(&home) { format!("Updated this device to {}, and restarting it.", thing.commit) } else { format!("Updated this device to {}. Nothing was running here to restart.", thing.commit) })
    // The program may be the newest while the oned running is still the one before it: an update before this couldn't
    // stop it. Update must never say "current" and leave that as it is.
    } else if std::fs::read_to_string(home.join(RUNNING)).unwrap_or_default().trim() != BUILD && restart_oned(&home) {
        Some(format!("This device's program is on {BUILD} already; restarting it, which still runs the build before."))
    } else {
        None
    };
    let window = match record.get(&Thing::app_name(TARGET)) {
        Some(app) => mac::window(&home, &address, app)?.then(|| format!("Its window is on {} now.", app.commit)),
        None => None,
    };
    Ok(match (said, window) {
        (None, None) => format!("This device is current, on {BUILD}. The newest published is {}.", thing.commit),
        (said, window) => [said, window].into_iter().flatten().collect::<Vec<_>>().join(" "),
    })
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
