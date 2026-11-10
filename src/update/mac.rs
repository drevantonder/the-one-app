//! A Mac takes its update as a whole app: `app/<target>` in the record is the zip one Mac built and published
//! (dev/skills/keep-mac-current). It's downloaded, checked and unpacked here, so anything wrong is said at once and
//! changes nothing; then `install-app.sh` replaces the app this program runs from and restarts it. That stops this
//! program's oned and the window, so it runs apart from them, and this returns without waiting for it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};

use crate::config::{CLI, COMMITTED, NAME, TARGET};
use crate::latest::{self, Thing};
use crate::release::Version;

/// The folder in a device's home an update works in: the app it unpacks, the installer and what it said.
const FOLDER: &str = "update";
/// What the installer says when it can't finish, for `status` to show (`why`).
const WHY: &str = "why";
const INSTALLER: &str = include_str!("../../client/native/mac/install-app.sh");

/// Why the last install stopped, if it did.
pub fn why(home: &Path) -> Option<String> {
    std::fs::read_to_string(home.join(FOLDER).join(WHY)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Downloads `thing`, the app's zip at `address`, checks that what's in it is the app the record names, and starts
/// installing it. → once it's started.
pub fn install(home: &Path, address: &str, thing: &Thing) -> Result<()> {
    let dir = home.join(FOLDER);
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::remove_file(dir.join(WHY));
    let bytes = latest::download(address, thing)?;
    // Named for this process, since the node's ask and a person can both be updating at once.
    let unpacked = dir.join(format!("fresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&unpacked);
    let app = unpack(&bytes, &dir.join(format!("app-{}.zip", std::process::id())), &unpacked).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&unpacked);
    })?;
    check(&app, &thing.commit).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&unpacked);
    })?;
    // Written out of the app that's about to be replaced, which the script itself does.
    let installer = dir.join("install-app.sh");
    std::fs::write(&installer, INSTALLER).context("write the installer")?;
    let log = std::fs::File::create(dir.join("install.log")).context("write the installer's log")?;
    let mut install = Command::new("/bin/bash");
    install
        .args(["-c", "/bin/bash \"$0\" \"$1\" && rm -rf \"$2\"", &installer.to_string_lossy(), &app.to_string_lossy(), &unpacked.to_string_lossy()])
        .env("CONFIG_NAME", NAME)
        .env("CONFIG_CLI", CLI)
        .env("INSTALL_WHY", dir.join(WHY))
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // Its own process group, so what stops oned and the app doesn't stop it; the app it was unpacked from goes when it's done.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut install, 0);
    install.spawn().context("start the installer")?;
    Ok(())
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

/// Whether the app's own program says it's `commit`, built for this Mac and no older than this one, as a replaced
/// program is checked (`release::install_commit`): the record's word isn't signed for that.
fn check(app: &Path, commit: &str) -> Result<()> {
    let program = app.join("Contents/MacOS").join(CLI);
    let version = Version::of(&program);
    ensure!(
        version.as_ref().is_some_and(|v| v.commit == commit && v.target == TARGET && v.committed >= COMMITTED),
        "the app says it's {}, not {} for {}",
        version.map_or("nothing".into(), |v| v.json()),
        commit,
        TARGET
    );
    Ok(())
}
