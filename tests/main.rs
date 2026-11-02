//! Every test of the crate's, in one binary.

mod contracts;
mod e2e;
mod kernel;
mod sim;

use std::path::{Path, PathBuf};

/// A new folder for a test, in one of this run's own: in memory where the system has a place for it, since nodes sync
/// each write to disk, which on a busy machine takes seconds. A run leaves its folder; the next removes those of runs
/// that have ended.
fn temp(name: &str) -> PathBuf {
    static RUN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let run = RUN.get_or_init(|| {
        let shm = Path::new("/dev/shm");
        let base = if shm.is_dir() { shm.to_path_buf() } else { std::env::temp_dir() };
        for entry in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.strip_prefix("one-tests-")?.parse::<i32>().ok()) else { continue };
            // SAFETY: signal 0 only asks whether the process is there.
            if unsafe { libc::kill(pid, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        base.join(format!("one-tests-{}", std::process::id()))
    });
    let dir = run.join(format!("{name}-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
