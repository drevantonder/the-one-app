//! This checkout's own build made again with the web client or the Cloudflare node it was built without.

use std::path::Path;

use anyhow::Result;
use one::config::CLI;

/// For `throwaway client web` from this checkout's own debug build: the web client's WebAssembly built, and this build made
/// again with it embedded from target/client, as every build of this checkout does from then on (the gateway serves what its build holds), as `{CLI} update` does for a release but at dev
/// speed: no release build, and cargo keeps what hasn't changed, so a run with nothing changed does nothing here. The
/// throwaway network's programs run the old build, so the command is run again as the new build, which starts it again (when
/// no client session is running).
pub(crate) fn web_client_built(checkout: &Path, rest: &[String]) -> Result<()> {
    let own = checkout.join("target/debug").join(env!("CARGO_BIN_NAME"));
    if std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) != own.canonicalize().ok() {
        return Ok(());
    }
    eprintln!("Building the web client's WebAssembly (the first time takes minutes; cargo keeps what hasn't changed)…");
    let built = crate::build::client(checkout, &checkout.join(one::config::CONFIG_FILE))?;
    let held = |name: &str, bytes: &[u8]| std::fs::read(built.join(name)).is_ok_and(|file| file == bytes);
    if !one::config::CLIENT.is_empty() && one::config::CLIENT.iter().all(|(name, bytes)| held(name, bytes)) {
        return Ok(());
    }
    rebuilt(checkout, rest, &format!("target/debug/{CLI} again with the web client"))
}

/// For `throwaway cloudflare node deploy` from this checkout's own debug build: the Cloudflare node's WebAssembly built (as a dev
/// build, not release) into target/cloudflare-node, as it is now (cargo keeps what hasn't changed), and this build made again
/// with it if it holds another, as `throwaway client web` does with the client's, so the two builds agree: every build of
/// this checkout holds both once they're there, and the throwaway network's relay (which names the Cloudflare node) starts for
/// any of them.
pub(crate) fn cloudflare_node_built(checkout: &Path, rest: &[String]) -> Result<()> {
    let own = checkout.join("target/debug").join(env!("CARGO_BIN_NAME"));
    if std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) != own.canonicalize().ok() {
        return Ok(());
    }
    eprintln!("Building the Cloudflare node's WebAssembly (the first time takes minutes; cargo keeps what hasn't changed)…");
    let built = crate::build::cloudflare_node(checkout, &checkout.join(one::config::CONFIG_FILE), "dev")?;
    let held = |name: &str, bytes: &[u8]| std::fs::read(built.join(name)).is_ok_and(|file| file == bytes);
    if !one::config::CLOUDFLARE_NODE.is_empty() && one::config::CLOUDFLARE_NODE.iter().all(|(name, bytes)| held(name, bytes)) {
        return Ok(());
    }
    rebuilt(checkout, rest, &format!("target/debug/{CLI} again with the Cloudflare node"))
}

/// Builds this checkout's debug build again, which takes the web client and Cloudflare node from target/ (build.rs, what
/// `cargo test` does too, so they build the same one) and watches the files there; the first time, it's watching none,
/// so touching build.rs has it look. The throwaway network's programs run the old build, so the command is run again as the new
/// build, which starts them again (when no client session is running).
pub(crate) fn rebuilt(checkout: &Path, rest: &[String], what: &str) -> Result<()> {
    let own = checkout.join("target/debug").join(env!("CARGO_BIN_NAME"));
    let again = format!("{}_THROWAWAY_REBUILT", CLI.to_uppercase());
    anyhow::ensure!(std::env::var_os(&again).is_none(), "{CLI} was built again and doesn't hold what it was built with: {what}");
    eprintln!("Building {what} (dev profile, not release)…");
    std::fs::File::options().write(true).open(checkout.join("build.rs"))?.set_modified(std::time::SystemTime::now())?;
    crate::build::quietly(std::process::Command::new("nice").arg(crate::build::cargo()).current_dir(checkout).env_remove("CLIENT").env_remove("CLOUDFLARE_NODE").arg("build"))?;
    use std::os::unix::process::CommandExt;
    Err(std::process::Command::new(&own).arg("throwaway").args(rest).env(&again, "1").exec().into())
}
