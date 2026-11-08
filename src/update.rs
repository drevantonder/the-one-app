//! `update` and `release`: the tip of main, built on your own devices and run on every one of them.
//! What runs anywhere is only ever what's on main.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use one::config::{CLI, CONFIG_FILE, CONFIG_REPO, NAME, RELEASE_KEY, RELEASE_SIGNER, REPO, TARGET};
use one::latest::{Latest, Thing};
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
            ("update".into(), "build main's tip (tested before it was pushed) and publish it: every device takes it itself".into()),
            ("release <vX.Y.Z>".into(), "the same, signed off and published on GitHub for the installer".into()),
            ("publish [binary]".into(), "sign and publish a build you made; every device on its platform updates".into()),
            ("web bundle [--out <folder>]".into(), "write this build's web client as a bundle for the native apps, ready to sign".into()),
            ("update mac-job".into(), format!("the shell that sets up a Mac to take its own updates: {CLI} update mac-job | ssh <mac> sh, once, for a Mac too old to have the job its app starts")),
            ("publish-app --zip <zip> [--config <commit>]".into(), "publish this Mac's built app, so every Mac downloads it instead of building it".into()),
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
                ["update", "mac-job"] => print!("{}", one::latest::mac_job(&crate::node().state()?.address().context("this network has no address yet")?)),
                ["publish-app"] => publish_app(Path::new(g.flag("zip").context("give the app with --zip")?), g.flag("config").unwrap_or("-"))?,
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Builds the tip of main and publishes it; with a tag, also on GitHub. It only publishes (to the record at the network's
/// address, `one::latest`): each device takes it itself, by its node or its Mac job, whenever it's up. It sends out only what
/// changed since the last update here: apps by their deploy, and the node and services built and published, then the Workers
/// on Cloudflare (and the Cloudflare node with them), deployed from their one home (the first to deploy them: no other
/// does) once it takes this build. Beside it runs `after-update` in this device's home, if there is one: what else is built
/// from main here, like another custom build's update.
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
    // What the last update here sent out.
    let done = cache().join("updated");
    let last = std::fs::read_to_string(&done).ok();
    let released = services_released();
    let unreleased = services(&src).iter().any(|n| !released.contains_key(n));
    if tag.is_none() && last.as_deref() == Some(this.as_str()) && !unreleased {
        println!("Already published {short}, main's tip: the last update here sent it out.");
        return Ok(());
    }
    if let Some(tag) = tag {
        ensure!(!succeeds(Command::new("git").current_dir(&src).args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])), "{tag} already exists");
    }
    let mut parts = last.as_deref().map_or_else(Parts::everything, |last| Parts::since(&src, last, &this));
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
    let sends = parts.node || parts.mac || !parts.apps.is_empty() || !parts.services.is_empty();
    let after = if sends && tag.is_none() { after_update()? } else { None };
    let mut missed = false;
    let moved = retire_gone(&src, &parts.renamed)?;
    if parts.mac {
        publish_source(&src, &commit, config_commit.as_deref().unwrap_or("-"), tag.is_some(), &release_key()?)?;
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
        if !missed {
            std::fs::write(&done, &this)?;
        }
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
    // This device's own node takes it too, and the built-in apps are deployed from its build.
    let here: Vec<Value> = devices().into_iter().filter(|d| d["here"] == true).collect();
    let mut failed = !publish_services(&src, &parts.services, &key, &commit);
    let published = timed("published the node", || build::publish(&crate::node(), &binary, &key, &version, None))?;
    println!("{}", build::published_said(&published, None));
    let started_at = Instant::now();
    failed |= timed("waited for this device", || await_builds(&here, short, started_at, Duration::from_secs(180)));
    if let Err(err) = timed("deployed the built-in apps", deploy_built_in) {
        failed = true;
        println!("  couldn't deploy the new versions of the built-in apps: {err:#}");
    }
    add_where(&moved);
    let cloudflare_failed = timed("deployed to Cloudflare", || deploy_cloudflare(short));
    if let Err(err) = timed("published the web client", || publish_web(&binary, &key, &version, config_commit.as_deref().unwrap_or("-"))) {
        failed = true;
        println!("  couldn't publish the web client for the native apps: {err:#}");
    }

    println!("Published {short}: every device takes it itself. {CLI} status --until {short} waits for the online ones, and says about the rest.");
    let Some(tag) = tag else {
        let after = finish(after);
        ensure!(!failed, "this device didn't take {short}");
        ensure!(!cloudflare_failed, "published {short}, but it isn't deployed to Cloudflare");
        std::fs::write(&done, &this)?;
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
    // The Mac app, once a Mac that holds the key has built and published it (`source-release`, which notarizes it).
    match timed("waited for a Mac's app", || await_app(&commit, &key_address(), Duration::from_secs(30 * 60))) {
        Ok(Some((thing, address))) => {
            let zip = app_zip();
            std::fs::write(dir.join(&zip), one::latest::download(&address, &thing)?)?;
            assets.push(zip);
        }
        Ok(None) => println!("No Mac published the app for {short}, so {tag} has none."),
        Err(err) => println!("  couldn't get a Mac's app: {err:#}"),
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
    ensure!(!failed, "released {tag}, but this device didn't take it");
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

fn key_address() -> String {
    crate::node().state().ok().and_then(|s| s.address().map(str::to_string)).unwrap_or_default()
}

/// Waits for a Mac to publish its app built from `commit`: → it, and where it's read from.
fn await_app(commit: &str, address: &str, limit: Duration) -> Result<Option<(Thing, String)>> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(text) = one::latest::fetch(address)?
            && let Some(thing) = Latest::read(RELEASE_KEY, &text)?.things.into_iter().find(|t| t.name.starts_with("app/") && commit.starts_with(&t.commit))
        {
            return Ok(Some((thing, address.to_string())));
        }
        std::thread::sleep(Duration::from_secs(20));
    }
    Ok(None)
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
/// if none is. A release's asks for its app notarized, which is every release's.
fn publish_source(src: &Path, commit: &str, config: &str, release: bool, key: &ssh_key::PrivateKey) -> Result<()> {
    let committed = git(src, &["log", "-1", "--format=%ct", commit])?.trim().parse().context("the commit's date")?;
    let version = Version { commit: commit.into(), target: "-".into(), committed };
    let mut things: Vec<(String, Option<PathBuf>, Version, String)> = vec![("source".into(), None, version.clone(), config.into())];
    if release {
        things.push(("source-release".into(), None, version, config.into()));
    }
    build::publish_latest(&crate::node(), key, &things)
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

/// This build's web client written to `dir` as `<commit>.bundle`, not yet signed: gzip, for `update` and `release` to sign and publish.
fn web_bundle(dir: &Path) -> Result<PathBuf> {
    let bundle = one::web::bundle::Bundle::of_this_build(one::config::BUILD, one::config::COMMITTED);
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.bundle", one::config::BUILD));
    std::fs::write(&path, bundle.packed()?)?;
    Ok(path)
}

/// The web client of the build at `binary`, signed with the release key and named in the record the native apps read,
/// unless it's what was last published from here: an update that didn't change it costs a phone nothing.
fn publish_web(binary: &Path, key: &ssh_key::PrivateKey, version: &Version, config: &str) -> Result<()> {
    let dir = cache().join("web");
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(binary).args(["web", "bundle", "--out"]).arg(&dir).output().context("run the new build")?;
    ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    let packed = std::fs::read(dir.join(format!("{}.bundle", version.commit)))?;
    let last = cache().join("published-web");
    if std::fs::read(&last).is_ok_and(|was| was == packed) {
        println!("  the web client hasn't changed");
        return Ok(());
    }
    let file = dir.join(format!("{}.signed", version.commit));
    std::fs::write(&file, one::web::bundle::signed(key, &packed)?)?;
    build::publish_latest(&crate::node(), key, &[("web".into(), Some(file), version.clone(), config.into())])?;
    std::fs::write(&last, packed)?;
    Ok(())
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
