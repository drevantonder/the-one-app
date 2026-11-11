//! A Mac's window: the app the record names beside the program (`app/<target>`, the zip one Mac built and published,
//! .agents/skills/keep-mac-current), installed where `Mode::mac_app` says. It carries no background program, so taking it
//! stops nothing but the window: it's downloaded, checked and unpacked first, so anything wrong is said at once and
//! changes nothing, then put in place of the one there, which is closed first and opened again if it was open.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use ::custom_build::config::{CLI, COMMITTED, NAME};
use crate::latest::{self, Thing};

use super::FOLDER;

/// Takes the window `thing` names, at `address`, unless it's the one installed or older than this program. → whether it
/// put it in place.
pub fn window(home: &Path, address: &str, thing: &Thing) -> Result<bool> {
    let installed = platform::mode::here().mac_app();
    if thing.committed < COMMITTED || built(&installed).is_some_and(|b| b == thing.commit) {
        return Ok(false);
    }
    let dir = home.join(FOLDER);
    std::fs::create_dir_all(&dir)?;
    let bytes = latest::download(address, thing)?;
    // Named for this process, since a person and an Update pressed elsewhere can both be updating at once.
    let unpacked = dir.join(format!("window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&unpacked);
    let done = (|| {
        let app = unpack(&bytes, &dir.join(format!("window-{}.zip", std::process::id())), &unpacked)?;
        let says = built(&app);
        ensure!(says.as_deref() == Some(thing.commit.as_str()), "the app says it's {}, not {}", says.as_deref().unwrap_or("nothing"), thing.commit);
        let open = !running(&installed).is_empty();
        close(&installed);
        replace(&app, &installed)?;
        if open {
            // Opened as a person would: it reads where it is and who it's for from itself.
            let _ = Command::new("open").arg(&installed).status();
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&unpacked);
    done.map(|()| true)
}

/// The build a window says it is: `Build` in its Info.plist (native-apps/mac/build.sh), what the record names it.
fn built(app: &Path) -> Option<String> {
    let out = Command::new("plutil").args(["-extract", "Build", "raw", "-o", "-"]).arg(app.join("Contents/Info.plist")).output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|b| !b.is_empty())
}

/// The zip's one app, unpacked in `into`. (`ditto` is the Mac's own, which keeps what a signed app is made of.)
fn unpack(zip: &[u8], at: &Path, into: &Path) -> Result<PathBuf> {
    std::fs::write(at, zip).context("keep the download")?;
    let done = Command::new("ditto").arg("-x").arg("-k").arg(at).arg(into).output();
    let _ = std::fs::remove_file(at);
    let done = done.context("unpack the download")?;
    ensure!(done.status.success(), "couldn't unpack the download: {}", String::from_utf8_lossy(&done.stderr).trim());
    let mut apps: Vec<PathBuf> = std::fs::read_dir(into)?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "app")).collect();
    match apps.len() {
        1 => Ok(apps.remove(0)),
        n => bail!("what's published has {n} apps in it, not one"),
    }
}

/// The ids of the window's processes running from `app`.
fn running(app: &Path) -> Vec<String> {
    let program = app.join("Contents/MacOS").join(NAME);
    let out = Command::new("pgrep").arg("-f").arg(format!("^{}$", program.display())).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().map(str::to_string).collect()).unwrap_or_default()
}

/// Closes the window running from `app`: asked, then made to if it hasn't in ten seconds.
fn close(app: &Path) {
    for signal in ["-TERM", "-KILL"] {
        let pids = running(app);
        if pids.is_empty() {
            return;
        }
        let _ = Command::new("kill").arg(signal).args(&pids).status();
        for _ in 0..100 {
            if running(app).is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    eprintln!("{CLI}: the window at {} didn't close", app.display());
}

/// Puts `new` at `at`: beside it, then swapped in by renames, so the path holds a whole app however this ends.
fn replace(new: &Path, at: &Path) -> Result<()> {
    let folder = at.parent().context("where the app goes")?;
    std::fs::create_dir_all(folder)?;
    let (fresh, old) = (folder.join(format!(".{NAME}.{}.new", std::process::id())), folder.join(format!(".{NAME}.{}.old", std::process::id())));
    std::fs::rename(new, &fresh).with_context(|| format!("put the new app beside {}", at.display()))?;
    if at.exists() {
        std::fs::rename(at, &old).with_context(|| format!("move {} aside", at.display()))?;
    }
    std::fs::rename(&fresh, at).with_context(|| format!("put the new app at {}", at.display()))?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}
