//! `update` and `release`: the tip of main, built on your own devices and run on every one of them.
//! What runs anywhere is only ever what's on main.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use one::config::{CLI, CONFIG_FILE, CONFIG_REPO, NAME, RELEASE_KEY, RELEASE_SIGNER, REPO, TARGET};
use one::release::Version;
use one::{cache, release};
use serde_json::{Value, json};

use crate::build::{self, PLATFORMS, Platform, cargo, git, quietly, show, succeeds};

mod apps;
mod cloudflare;
mod mac;
mod parts;
mod services;
mod wait;

use apps::*;
use cloudflare::*;
use mac::*;
use parts::*;
use services::*;
pub use wait::wait;
use wait::*;
pub use services::try_service;

/// `update`, `release` and `publish`, and putting your checkout's build of a service on a device.
pub fn commands() -> one::cli::Command {
    one::cli::Command {
        help: vec![
            ("update".into(), "build main's tip (tested before it was pushed) on this machine and the Macs, and run it everywhere".into()),
            ("release <vX.Y.Z>".into(), "the same, signed off and published on GitHub for the installer".into()),
            ("publish [binary]".into(), "sign and publish a build you made; every device on its platform updates".into()),
            ("web bundle [--out <folder>]".into(), "write this build's web client as a bundle for the native apps, ready to sign".into()),
            ("services try apps/<name> [--device <name>]".into(), "put your checkout's build of an app's service on a device of yours (here by default, else over ssh by its name) and switch it on, until a newer one is published".into()),
        ],
        run: |g| {
            match g.words().as_slice() {
                ["update"] => run(None)?,
                ["release", tag] => run(Some(tag))?,
                ["publish", rest @ ..] => {
                    let path = match rest {
                        [path] => std::fs::canonicalize(path)?,
                        _ => std::env::current_exe()?,
                    };
                    publish(g, &path, &release_key()?)?;
                }
                ["services", "try", dir] => try_service(Path::new(dir), g.flag("device"))?,
                ["web", "bundle"] => println!("{}", web_bundle(Path::new(g.flag("out").unwrap_or(".")))?.display()),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Builds the tip of main and has every device run it; with a tag, also publishes it on GitHub.
/// An update sends out only what changed since the last one here: apps by their deploy, the Mac app by each Mac building
/// it at once, then the iPhone app for the iPhone it reaches, and the node by building it, which every Linux device then
/// takes, and which deploys the Workers on Cloudflare (and the Cloudflare node with it) from the device they're deployed from (the first to
/// deploy them: no other does). Beside it runs `after-update` in this device's home, if there is one: what else is built from main here, like
/// another custom build's update.
pub fn run(tag: Option<&str>) -> Result<()> {
    ensure!(!REPO.is_empty(), "this custom build names no repo, so it has nothing to update from");
    ensure!(cfg!(target_os = "linux"), "run this on one of your Linux devices");
    // It runs from its own cache, not the folder it was started in, which (an agent's worktree) may be gone before it ends.
    ensure!(cache().is_absolute(), "give {} as a full path", one::config::env("HOME"));
    std::fs::create_dir_all(cache())?;
    std::env::set_current_dir(cache())?;
    if let Some(tag) = tag {
        ensure!(version_tag(tag), "a release is named like v1.2.3");
        ensure!(build::installer_is_ours(), "install.sh installs another custom build, so {NAME} isn't released on GitHub; {CLI} update runs main on its devices");
        ensure!(succeeds(Command::new("gh").args(["auth", "status"])), "sign in to GitHub first: gh auth login");
    }
    // Updates here take turns: they share one checkout, and each would test one commit and send out another.
    let _turn = turn()?;
    let src = timed("fetched main", checkout)?;
    let (config, config_commit) = config(&src)?;
    let commit = git(&src, &["rev-parse", "HEAD"])?;
    let short = commit[..12].to_string();
    let short = short.as_str();
    // Whoever pushed since another update began is in what it sent out, if it got to their commit.
    let this = format!("{commit} {}", config_commit.as_deref().unwrap_or(""));
    // What the last update here sent everywhere, and, apart, what it sent the Macs: a Mac that was off doesn't hold back
    // the rest, which would send everything since again each time.
    let done = cache().join("updated");
    let last = std::fs::read_to_string(&done).ok();
    let mac_done = cache().join("updated-mac");
    let mac_last = std::fs::read_to_string(&mac_done).ok().or_else(|| last.clone());
    let released = services_released();
    let unreleased = services(&src).iter().any(|n| !released.contains_key(n));
    if tag.is_none() && last.as_deref() == Some(this.as_str()) && mac_last.as_deref() == Some(this.as_str()) && !unreleased {
        println!("Every device already runs {short}, main's tip: the last update here sent it out.");
        return Ok(());
    }
    if let Some(tag) = tag {
        ensure!(!succeeds(Command::new("git").current_dir(&src).args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])), "{tag} already exists");
    }
    let mut parts = last.as_deref().map_or_else(Parts::everything, |last| Parts::since(&src, last, &this));
    parts.mac = mac_last.as_deref().map_or_else(Parts::everything, |last| Parts::since(&src, last, &this)).mac;
    if tag.is_some() {
        (parts.node, parts.mac) = (true, true);
    }
    for name in services(&src) {
        let rust = service_dir(&src, &name).join("Cargo.toml").is_file();
        if (tag.is_some() || last.is_none() || !released.contains_key(&name) || (rust && parts.service)) && !parts.services.contains(&name) {
            parts.services.push(name);
        }
    }
    parts.services.retain(|n| services(&src).contains(n));
    let macs: Vec<Value> =
        if parts.mac { devices().into_iter().filter(|d| d["target"].as_str().is_some_and(|t| Platform::is_mac(t))).collect() } else { Vec::new() };

    let sends = parts.node || parts.mac || !parts.apps.is_empty() || !parts.services.is_empty();
    let after = if sends && tag.is_none() { after_update()? } else { None };
    // The Macs start first, since their builds take longest: an update's in the background, and a release's with what it
    // publishes, which it waits for.
    let (mut missed, mut missed_macs) = (false, false);
    let mut started = Vec::new();
    if tag.is_some() {
        for mac in &macs {
            let name = mac["name"].as_str().unwrap_or("");
            match mac_shell(mac, &mac_job(&commit, config_commit.as_deref()), 60) {
                Ok(r) if r["exit"] == 0 => started.push((mac.clone(), Instant::now())),
                Ok(r) => println!("  {name}: couldn't start its build: {}", r["stderr"].as_str().unwrap_or("").trim()),
                Err(err) => println!("  {name}: couldn't start its build: {err:#}"),
            }
        }
    } else {
        missed_macs = follow(&macs, &commit, config_commit.as_deref());
    }
    let mark_macs = |missed_macs: bool| {
        if !missed_macs {
            let _ = std::fs::write(&mac_done, &this);
        }
    };
    let moved = retire_gone(&src, &parts.renamed)?;
    if parts.mac {
        publish_source(&src, &commit, config_commit.as_deref().unwrap_or("-"), &release_key()?)?;
    }
    if !parts.node {
        deploy_apps(&src, &parts.apps)?;
        add_where(&moved);
        if !parts.services.is_empty() {
            missed |= !publish_services(&src, &parts.services, &release_key()?, &commit);
        }
        if !sends {
            println!("Nothing to send out: what changed since the last update here isn't in any build.");
        }
        if !macs.is_empty() {
            says_wait(short);
        }
        if !missed {
            std::fs::write(&done, &this)?;
        }
        mark_macs(missed_macs);
        return finish(after);
    }

    let key = release_key()?;
    // Its commits were tested before they were pushed.
    println!("Building {short}");
    let mut node = Command::new(cargo());
    node.current_dir(&src).env("CONFIG", &config).args(["build", "--release"]);
    // Every release carries the Cloudflare node: one without it couldn't deploy to Cloudflare.
    node.env("CLOUDFLARE_NODE", timed("built the Cloudflare node", || build::cloudflare_node(&src, &config, "release")).context("couldn't build the Cloudflare node, so nothing was sent out")?);
    // And the web client's way to nodes: one without it couldn't reach them.
    node.env("CLIENT", timed("built the web client", || build::client(&src, &config)).context("couldn't build the web client's WebAssembly, so nothing was sent out")?);
    timed("built the node", || quietly(&mut node))?;

    let binary = src.join("target/release").join(env!("CARGO_BIN_NAME"));
    let version = Version::of(&binary).context("ask the new build what it is")?;
    let short = version.commit.as_str();
    // Which devices to wait for, asked before this device's own node restarts on the new build.
    let linux: Vec<Value> = devices().into_iter().filter(|d| d["target"] == TARGET).collect();
    // Services first, so a node that no longer carries one finds it out.
    let mut failed = !publish_services(&src, &parts.services, &key, &commit);
    if !linux.is_empty() && linux.iter().all(|d| d["build"] == short) {
        println!("  every Linux device already runs {short}");
    } else {
        let published = timed("published the node", || build::publish(&crate::node(), &binary, &key, &version, None))?;
        println!("{}", build::published_said(&published, None));
    }
    let started_at = Instant::now();
    failed |= timed("waited for the Linux devices", || await_builds(&linux, short, started_at, Duration::from_secs(180)));
    if let Err(err) = timed("deployed the built-in apps", deploy_built_in) {
        failed = true;
        println!("  couldn't deploy the new versions of the built-in apps: {err:#}");
    }
    add_where(&moved);
    let cloudflare_failed = timed("deployed to Cloudflare", || deploy_cloudflare(short));

    let Some(tag) = tag else {
        if !macs.is_empty() {
            says_wait(short);
        }
        let after = finish(after);
        ensure!(!failed, "not every device runs {short}");
        ensure!(!cloudflare_failed, "every device runs {short}, but it isn't deployed to Cloudflare");
        std::fs::write(&done, &this)?;
        mark_macs(missed_macs);
        return after;
    };
    let mut assets = Vec::new();
    let dir = cache().join("release").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let name = format!("{CLI}-{TARGET}");
    std::fs::copy(&binary, dir.join(&name))?;
    assets.push(name);
    // state::install_line() fetches it from the latest release.
    std::fs::write(dir.join("install.sh"), include_str!("../install.sh"))?;
    assets.push("install.sh".into());
    // The native app for this platform (client/native, its own workspace), which install.sh puts on a desktop.
    if cfg!(target_os = "linux") {
        println!("Building the Linux app");
        let native = src.join("client/native");
        match quietly(Command::new(cargo()).current_dir(&native).env("CONFIG", &config).args(["build", "--release"])) {
            Ok(()) => {
                let name = format!("{CLI}-app-{TARGET}");
                std::fs::copy(native.join("target/release/one-native"), dir.join(&name))?;
                assets.push(name);
            }
            Err(err) => {
                failed = true;
                println!("  couldn't build the Linux app: {err:#}");
            }
        }
    }
    for (mac, since) in &started {
        let name = mac["name"].as_str().unwrap_or("");
        match await_mac(mac) {
            Ok((made, warnings)) => {
                for w in warnings {
                    println!("  {name}: {w}");
                }
                failed |= await_builds(std::slice::from_ref(mac), short, *since, Duration::from_secs(120));
                for (hash, asset) in made {
                    let bytes = blob(&hash).with_context(|| format!("get {asset} from {name}"))?;
                    std::fs::write(dir.join(&asset), bytes)?;
                    assets.push(asset);
                }
            }
            Err(err) => {
                failed = true;
                println!("  {name}: {err:#}");
            }
        }
    }
    for p in &PLATFORMS {
        if !assets.contains(&p.asset()) {
            println!("No device here builds {}, so {tag} has nothing for it.", p.target);
        }
    }
    for asset in std::mem::take(&mut assets) {
        std::fs::write(dir.join(format!("{asset}.sig")), release::sign(&key, &std::fs::read(dir.join(&asset))?)?)?;
        assets.extend([format!("{asset}.sig"), asset]);
    }
    println!("Publishing {tag} on GitHub");
    let mut gh = Command::new("gh");
    gh.current_dir(&dir).args(["release", "create", tag, "--repo", REPO, "--target", &commit, "--generate-notes"]).args(&assets);
    show(&mut gh)?;
    ensure!(!failed, "released {tag}, but not every device runs it");
    ensure!(!cloudflare_failed, "released {tag}, but it isn't deployed to Cloudflare");
    Ok(())
}

/// This device's turn to update, held until it's dropped: other updates wait for it.
fn turn() -> Result<std::fs::File> {
    let asked = Instant::now();
    let file = turn_waiting()?;
    if asked.elapsed() >= Duration::from_secs(3) {
        println!("  waited {}s for the other update", asked.elapsed().as_secs());
    }
    Ok(file)
}

/// `f`, saying what it was and how long it took when that was a few seconds or more: where an update's time goes.
fn timed<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    if start.elapsed() >= Duration::from_secs(3) {
        println!("  {what}: {}s", start.elapsed().as_secs());
    }
    out
}

fn turn_waiting() -> Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    std::fs::create_dir_all(cache())?;
    let file = std::fs::File::create(cache().join("update.lock"))?;
    // SAFETY: flock on a descriptor this function owns; the lock goes when the file closes.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        println!("Another update is running here: waiting for it to end, then this one runs with main's tip (and sends nothing if that update already sent it).");
        ensure!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0, "couldn't wait for the other update: {}", std::io::Error::last_os_error());
    }
    Ok(file)
}

/// This device's `after-update`, running beside the update that started it: what it said is shown when it's done,
/// however the update ends.
struct After {
    child: Option<std::process::Child>,
    log: PathBuf,
}

impl After {
    fn wait(&mut self) -> bool {
        let Some(mut child) = self.child.take() else { return true };
        let status = child.wait();
        println!("{} said:", one::home().join("after-update").display());
        print!("{}", std::fs::read_to_string(&self.log).unwrap_or_default());
        let _ = std::fs::remove_file(&self.log);
        status.is_ok_and(|s| s.success())
    }
}

impl Drop for After {
    fn drop(&mut self) {
        if self.child.is_some() && !self.wait() {
            println!("after-update failed");
        }
    }
}

/// Starts this device's `after-update`, if it has one, for an update that sends something out: it builds from main as
/// this update does, so it needn't wait for it. What it says goes to a file, so it finishes even if this update stops.
fn after_update() -> Result<Option<After>> {
    let script = one::home().join("after-update");
    if !script.exists() {
        return Ok(None);
    }
    let log = cache().join(format!("after-update-{}.log", std::process::id()));
    println!("Running {} beside this, saying what it does in {}", script.display(), log.display());
    let out = std::fs::File::create(&log)?;
    let child = Command::new("sh").arg(&script).stdin(std::process::Stdio::null()).stdout(out.try_clone()?).stderr(out).spawn()?;
    Ok(Some(After { child: Some(child), log }))
}

fn finish(after: Option<After>) -> Result<()> {
    ensure!(timed("waited for after-update", || after.is_none_or(|mut after| after.wait())), "after-update failed");
    Ok(())
}

/// `v1.2.3`, or `v1.2.3-rc.1`.
fn version_tag(tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix('v') else { return false };
    let core = rest.split_once('-').map_or(rest, |(core, pre)| if pre.is_empty() { "" } else { core });
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// What the Mac app is published as.
fn app_zip() -> String {
    format!("{CLI}-macos.zip")
}

/// The release key's private half, from the publisher's password manager by way of the config's release_signer, kept
/// only in memory. It must be the key every build checks releases against.
pub fn release_key() -> Result<ssh_key::PrivateKey> {
    ensure!(!RELEASE_KEY.is_empty() && !RELEASE_SIGNER.is_empty(), "this custom build has no release key, so nothing it builds can be released");
    let out = Command::new("sh").args(["-c", RELEASE_SIGNER]).stdin(std::process::Stdio::null()).output().context("run release_signer")?;
    ensure!(
        out.status.success(),
        "couldn't get the release key with `{RELEASE_SIGNER}`: {}\nIs the password manager it reads reachable, and unlocked?",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let key = ssh_key::PrivateKey::from_openssh(&out.stdout).map_err(|_| anyhow::anyhow!("`{RELEASE_SIGNER}` didn't give an OpenSSH private key"))?;
    ensure!(key.public_key().key_data() == ssh_key::PublicKey::from_openssh(RELEASE_KEY)?.key_data(), "`{RELEASE_SIGNER}` gave a key, but not the release key in config.toml");
    Ok(key)
}

/// Says in the record that this commit is the one to build: every Mac takes the app published for it, or builds it itself
/// if none is.
fn publish_source(src: &Path, commit: &str, config: &str, key: &ssh_key::PrivateKey) -> Result<()> {
    let committed = git(src, &["log", "-1", "--format=%ct", commit])?.trim().parse().context("the commit's date")?;
    build::publish_latest(&crate::node(), key, &[("source".into(), None, Version { commit: commit.into(), target: "-".into(), committed }, config.into())])
}

/// A clean clone of the repo, kept in the cache, at the tip of main on GitHub. Build output stays, so builds are quick.
fn checkout() -> Result<PathBuf> {
    let dir = cache().join("src");
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(cache())?;
        show(Command::new("git").args(["clone", "-q", &format!("https://github.com/{REPO}.git")]).arg(&dir))?;
    }
    for args in [&["fetch", "-q", "--tags", "--force", "origin"][..], &["checkout", "-q", "--detach", "--force", "origin/main"], &["clean", "-qfdx", "-e", "/target"]] {
        show(Command::new("git").current_dir(&dir).args(args))?;
    }
    Ok(dir)
}

/// The config to build with: this checkout's own, or, for a custom build whose config lives in its own repo, that repo's
/// latest, fetched beside it; and then the commit it's at.
fn config(src: &Path) -> Result<(PathBuf, Option<String>)> {
    if CONFIG_REPO.is_empty() {
        let file = src.join(CONFIG_FILE);
        ensure!(file.exists(), "{} isn't there to build with", file.display());
        return Ok((file, None));
    }
    let dir = cache().join("config");
    if !dir.join(".git").exists() {
        show(Command::new("git").args(["clone", "-q", CONFIG_REPO]).arg(&dir))?;
    }
    for args in [&["fetch", "-q", "origin", "HEAD"][..], &["checkout", "-q", "--detach", "--force", "FETCH_HEAD"]] {
        show(Command::new("git").current_dir(&dir).args(args))?;
    }
    Ok((dir.join(CONFIG_FILE), Some(git(&dir, &["rev-parse", "HEAD"])?)))
}

/// A command in this machine's own sh, answered as `shell` answers: its exit and what it said.
fn run_here(command: &str) -> Result<Value> {
    let out = Command::new("sh").args(["-c", command]).output().context("run sh")?;
    Ok(json!({ "exit": out.status.code().unwrap_or(-1), "stdout": String::from_utf8_lossy(&out.stdout), "stderr": String::from_utf8_lossy(&out.stderr) }))
}

fn shell(device: &Value, command: &str, timeout: u64) -> Result<Value> {
    let r = crate::node().post("/v1/call", json!({ "device": device["name"], "service": "shell", "app": "shell", "function": "run", "input": { "command": command, "timeout": timeout } }))?;
    Ok(r["ok"].clone())
}

fn blob(hash: &str) -> Result<Vec<u8>> {
    let mut resp = crate::agent().get(format!("http://{}/v1/blobs/{hash}", one::api::addr())).call()?;
    Ok(resp.body_mut().with_config().limit(1 << 30).read_to_vec()?)
}

/// This build's web client written to `dir` as `<commit>.bundle`, not yet signed: gzip, for `update` and `release` to sign and publish.
fn web_bundle(dir: &Path) -> Result<PathBuf> {
    let bundle = one::web::bundle::Bundle::of_this_build(one::config::BUILD, one::config::COMMITTED);
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.bundle", one::config::BUILD));
    std::fs::write(&path, bundle.packed()?)?;
    Ok(path)
}

/// Publishes the binary at `path` as a release, signed with `key`: this one, or one built for another platform
/// (--target <triple> --commit <hash> --committed <seconds>), or a service's program (--service <name>, with those).
fn publish(g: &one::cli::Given, path: &Path, key: &ssh_key::PrivateKey) -> Result<()> {
    // The binary knows what it is; ask it, unless it's for another platform and can't run here, or a service's.
    let service = g.flag("service");
    let version = if service.is_none() { Version::of(path) } else { None };
    let flag = |name: &str| g.flag(name).map(str::to_string);
    let version = Version {
        committed: g.flag("committed").and_then(|c| c.parse().ok()).or(version.as_ref().map(|v| v.committed)).context("say when the commit was made with --committed (seconds since 1970)")?,
        commit: flag("commit").or(version.as_ref().map(|v| v.commit.clone())).context("say which commit this is with --commit")?,
        target: flag("target").or(version.as_ref().map(|v| v.target.clone())).context("say which platform this is for with --target")?,
    };
    println!("{}", build::published_said(&build::publish(g.node, path, key, &version, service)?, service));
    Ok(())
}
