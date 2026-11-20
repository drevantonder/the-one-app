//! Where a window's last frame, and an app's window's place on the screen, are kept.

/// Where a window's last frame is kept (`WindowOptions::snapshot`), named by whose it is, their network and what it
/// shows for whom (`entrypoint`, `subject`), so it's never shown to another person. A new build of its page, or of
/// this program, shows the last one until its page has started, as iOS shows an app's last screen after an update,
/// where every deploy and release had it open on nothing. None while no one's signed in. They all go as the device
/// signs out (`forget_snapshots`).
pub(super) fn snapshot_file(entrypoint: &str, subject: Option<&str>) -> Option<std::path::PathBuf> {
    use sha2::{Digest, Sha256};
    let client = &crate::views().ok()?.client;
    let who = client.who()?;
    let address = client.address().unwrap_or_default();
    let mut hash = Sha256::new();
    for part in [who.as_str(), &address, entrypoint, subject.unwrap_or_default()] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    Some(crate::app().shell.home().ok()?.join("snapshots").join(data_encoding::HEXLOWER.encode(&hash.finalize())))
}

/// Every window's last frame goes, as no one's signed in on the device.
pub(crate) fn forget_snapshots() {
    if let Ok(home) = crate::app().shell.home() {
        let _ = std::fs::remove_dir_all(home.join("snapshots"));
    }
}

/// Where an app's window is kept where it was left (`WindowOptions::frame`): a computer's app window's, by its app,
/// its page and who it's for.
pub(super) fn frame_file(launcher: bool, sheet: bool, entrypoint: &str, subject: Option<&str>) -> Option<std::path::PathBuf> {
    #[cfg(desktop)]
    if !launcher && !sheet {
        return crate::app().shell.home().ok().map(|home| home.join("windows").join(format!("{}.json", crate::shortcuts::app_id(entrypoint, subject))));
    }
    let _ = (launcher, sheet, entrypoint, subject);
    None
}
