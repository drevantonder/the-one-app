//! The Mac app of what's sent, built on a Mac over ssh and published from here: a Mac behind what the network reads
//! can't publish through its own app runtime, which the URL signer refuses, so the device with the release key does.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use ::custom_build::config::{CLI, CONFIG_FILE, CONFIG_REPO, REPO, RELEASE_KEY};
use oned_update::latest::Latest;

use crate::throwaway::shell_quote;

/// Builds the Mac app of the record's source on the Mac ssh reaches as `MAC` (`mac` if unset), in the checkout its agent
/// builds in (keep-mac-current), and publishes it from here, unless the record has a Mac's app of it already. A network
/// with no Mac ssh reaches gets its Mac app from a Mac's agent, as before. It ends by `deadline`.
pub(super) fn build_and_publish(deadline: Instant) -> Result<()> {
    let address = super::key_address();
    let text = oned_update::latest::fetch(&address)?.with_context(|| format!("nothing is published at {address}"))?;
    let record = Latest::read(RELEASE_KEY, &text)?;
    let Some(source) = record.things.iter().find(|t| t.name == "source") else { return Ok(()) };
    let (commit, short) = (source.commit.as_str(), &source.commit[..12.min(source.commit.len())]);
    if record.things.iter().any(|t| t.name.starts_with("app/") && !t.commit.is_empty() && source.commit.starts_with(&t.commit)) {
        println!("mac: a Mac's app of {short} is published already.");
        return Ok(());
    }
    let host = std::env::var("MAC").unwrap_or_else(|_| "mac".into());
    let ssh = |command: &str| Command::new("ssh").args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", &host, command]).stdin(Stdio::null()).output();
    if !ssh("true").is_ok_and(|o| o.status.success()) {
        println!("mac: no Mac answered ssh as {host} (MAC names another), so a Mac's agent builds its app (keep-mac-current).");
        return Ok(());
    }
    // The agent's checkout keeps the build's cache, too large to keep twice; a build of its running there is left be.
    let cache = format!("Library/Caches/{CLI}");
    let building = ssh("pgrep -f native-apps/mac/build.sh")?;
    ensure!(!building.status.success(), "{host} is building its app already (its agent, keep-mac-current): once it's done, {CLI}-dev update mac publishes it if it hasn't");
    let src = format!("{cache}/src");
    // Its config's artwork is named relative to it: a custom build's is its config repo, cloned there at its commit.
    let config = match Some(source.config.as_str()).filter(|c| *c != "-") {
        None => format!("$HOME/{src}/{CONFIG_FILE}"),
        Some(commit) => {
            let dir = format!("{cache}/config");
            ssh(&format!("[ -d {dir}/.git ] || git clone -q {} {dir}; cd {dir} && git fetch -q origin && git checkout -q --detach --force {commit}", shell_quote(CONFIG_REPO)))
                .ok()
                .filter(|o| o.status.success())
                .with_context(|| format!("couldn't put the config repo at {commit} on {host}"))?;
            format!("$HOME/{dir}/{CONFIG_FILE}")
        }
    };
    let build = format!(
        "export PATH=$HOME/.cargo/bin:/opt/homebrew/bin:$PATH; [ -d {src}/.git ] || git clone -q {repo} {src}; cd {src} && git fetch -q --tags --force origin && git checkout -q --detach --force {commit} && git clean -qfdx -e /target -e /native-apps/target && CONFIG={config} CARGO_BUILD_JOBS=$(( $(sysctl -n hw.ncpu) / 2 )) CARGO_TERM_COLOR=never nice -n 19 native-apps/disk.sh retry native-apps/mac/build.sh",
        repo = shell_quote(REPO),
    );
    println!("mac: building {short} on {host}…");
    let started = Instant::now();
    crate::build::build_on(&host, &build, &platform::cache().join("mac-build.log"), deadline)?;
    println!("mac: built on {host} in {}s.", started.elapsed().as_secs());
    super::publish_mac_from(&format!("{host}:{src}/native-apps/target/mac/built"), &source.config)
}

/// How long before the harness's cut-off a Mac build is stopped, for what follows it to finish.
pub(super) const AFTER: Duration = Duration::from_secs(90);
