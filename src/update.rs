//! `update` and `release`: the tip of main, built on your own devices and run on every one of them.
//! What runs anywhere is only ever what's on main.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use one::config::{CLI, CONFIG_FILE, CONFIG_REPO, NAME, RELEASE_KEY, RELEASE_SIGNER, REPO, TARGET};
use one::{cache, release};
use serde_json::{Value, json};

use crate::{get, post};

/// Platforms the installer knows. A release has each one some device here can build.
const PLATFORMS: [&str; 3] = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "aarch64-apple-darwin"];

/// Builds the tip of main and has every device run it; with a tag, also publishes it on GitHub.
/// An update sends out only what changed since the last one here: apps by their deploy, the Mac app by each Mac building
/// it at once, then the iPhone app for the iPhone it reaches, and the node by building it, which every Linux device then
/// takes, and which deploys the kernel (and the serverless node with it) from the device with the kernel's settings. Beside it runs `after-update` in this
/// device's home, if there is one: what else is built from main here, like another white label's update.
pub fn run(tag: Option<&str>) -> Result<()> {
    ensure!(!REPO.is_empty(), "this white label names no repo, so it has nothing to update from");
    ensure!(cfg!(target_os = "linux"), "run this on one of your Linux devices");
    // It runs from its own cache, not the folder it was started in, which (an agent's worktree) may be gone before it ends.
    ensure!(cache().is_absolute(), "give {}_HOME as a full path", CLI.to_uppercase());
    std::fs::create_dir_all(cache())?;
    std::env::set_current_dir(cache())?;
    if let Some(tag) = tag {
        ensure!(version_tag(tag), "a release is named like v1.2.3");
        ensure!(installer_is_ours(), "install.sh installs another white label, so {NAME} isn't released on GitHub; {CLI} update runs main on its devices");
        ensure!(quiet(Command::new("gh").args(["auth", "status"])), "sign in to GitHub first: gh auth login");
    }
    // Updates here take turns: they share one checkout, and each would test one commit and send out another.
    let _turn = turn()?;
    let src = checkout()?;
    let (config, config_commit) = config(&src)?;
    let commit = git(&src, &["rev-parse", "HEAD"])?;
    let short = commit[..12].to_string();
    let short = short.as_str();
    // Whoever pushed since another update began is in what it sent out, if it got to their commit.
    let this = format!("{commit} {}", config_commit.as_deref().unwrap_or(""));
    let done = cache().join("updated");
    let last = std::fs::read_to_string(&done).ok();
    if tag.is_none() && last.as_deref() == Some(this.as_str()) {
        println!("Every device already runs {short}, main's tip: the last update here sent it out.");
        return Ok(());
    }
    if let Some(tag) = tag {
        ensure!(!quiet(Command::new("git").current_dir(&src).args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])), "{tag} already exists");
    }
    let mut parts = last.as_deref().map_or_else(Parts::everything, |last| Parts::since(&src, last, &this));
    // A release sends all of it; the diff still says which apps' folders went and what a rename became.
    if tag.is_some() {
        (parts.node, parts.mac) = (true, true);
    }
    let macs: Vec<Value> =
        if parts.mac { devices().into_iter().filter(|d| d["target"].as_str().is_some_and(|t| t.ends_with("-apple-darwin"))).collect() } else { Vec::new() };

    let sends = parts.node || parts.mac || !parts.apps.is_empty();
    let after = if sends && tag.is_none() { after_update()? } else { None };
    // The Macs start first, since their builds take longest: an update's in the background, and a release's with what it
    // publishes, which it waits for.
    let mut missed = false;
    let mut started = Vec::new();
    if tag.is_some() {
        for mac in &macs {
            let name = mac["name"].as_str().unwrap_or("");
            if mac["online"] != true {
                println!("  {name}: offline; run this again when it's back");
                continue;
            }
            match shell(mac, &mac_job(&commit, config_commit.as_deref()), 60) {
                Ok(r) if r["exit"] == 0 => started.push((mac.clone(), Instant::now())),
                Ok(r) => println!("  {name}: couldn't start its build: {}", r["stderr"].as_str().unwrap_or("").trim()),
                Err(err) => println!("  {name}: couldn't start its build: {err:#}"),
            }
        }
    } else {
        missed = follow(&macs, &commit, config_commit.as_deref());
    }
    let moved = retire_gone(&src, &parts.renamed)?;
    if !parts.node {
        deploy_apps(&src, &parts.apps)?;
        add_where(&moved);
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
    let mut build = Command::new(cargo());
    build.current_dir(&src).env("CONFIG", &config).args(["build", "--release"]);
    // Every release carries the serverless node: one without it couldn't deploy the kernel.
    build.env("SERVERLESS", serverless(&src, &config).context("couldn't build the serverless node, so nothing was sent out")?);
    // And the web client's way to nodes: one without it couldn't reach them.
    build.env("CLIENT", client(&src, &config).context("couldn't build the web client's WebAssembly, so nothing was sent out")?);
    run_quiet(&mut build)?;

    let binary = src.join("target/release").join(env!("CARGO_BIN_NAME"));
    // The build devices say they run: main's commit, and a white label's config's.
    let version: Value = serde_json::from_slice(&Command::new(&binary).args(["version", "--json"]).output()?.stdout).context("ask the new build what it is")?;
    let short = version["commit"].as_str().context("the new build doesn't say what it is")?.to_string();
    let short = short.as_str();
    // Which devices to wait for, asked before this device's own node restarts on the new build.
    let linux: Vec<Value> = devices().into_iter().filter(|d| d["target"] == TARGET).collect();
    if !linux.is_empty() && linux.iter().all(|d| d["build"] == short) {
        println!("  every Linux device already runs {short}");
    } else {
        crate::publish(&binary, &Default::default(), &key)?;
    }
    let started_at = Instant::now();
    let mut failed = await_builds(&linux, short, started_at, Duration::from_secs(180));
    if let Err(err) = deploy_built_in() {
        failed = true;
        println!("  couldn't deploy the new versions of the built-in apps: {err:#}");
    }
    add_where(&moved);
    let kernel_failed = deploy_kernel(&linux, short);

    let Some(tag) = tag else {
        let after = finish(after);
        ensure!(!failed, "not every device runs {short}");
        ensure!(!kernel_failed, "every device runs {short}, but the kernel isn't deployed");
        if !missed {
            std::fs::write(&done, &this)?;
        }
        return after;
    };
    let mut assets = Vec::new();
    let dir = cache().join("release").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let name = format!("{CLI}-{TARGET}");
    std::fs::copy(&binary, dir.join(&name))?;
    assets.push(name);
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
    for p in PLATFORMS {
        if !assets.iter().any(|a| a.ends_with(p)) && !(p.ends_with("-apple-darwin") && assets.iter().any(|a| a == &app_zip())) {
            println!("No device here builds {p}, so {tag} has nothing for it.");
        }
    }
    // Each file comes with its signature, which the installer checks.
    for asset in std::mem::take(&mut assets) {
        std::fs::write(dir.join(format!("{asset}.sig")), release::sign(&key, &std::fs::read(dir.join(&asset))?)?)?;
        assets.extend([format!("{asset}.sig"), asset]);
    }
    println!("Publishing {tag} on GitHub");
    let mut gh = Command::new("gh");
    gh.current_dir(&dir).args(["release", "create", tag, "--repo", REPO, "--target", &commit, "--generate-notes"]).args(&assets);
    step(&mut gh)?;
    ensure!(!failed, "released {tag}, but not every device runs it");
    ensure!(!kernel_failed, "released {tag}, but the kernel isn't deployed");
    Ok(())
}

/// What a change to the repo has to send out.
#[derive(Debug, Default, PartialEq)]
struct Parts {
    /// The node: built, and taken by every device.
    node: bool,
    /// The Mac app, which each Mac builds.
    mac: bool,
    /// Built-in apps, by name, each deployed on its own.
    apps: Vec<String>,
    /// Built-in apps renamed: (old name, new name).
    renamed: Vec<(String, String)>,
}

impl Parts {
    fn everything() -> Parts {
        Parts { node: true, mac: true, ..Default::default() }
    }

    /// What changed from the commit `last` says (and its config's) to `this`'s.
    fn since(src: &Path, last: &str, this: &str) -> Parts {
        let (Some((was, was_config)), Some((now, now_config))) = (last.split_once(' '), this.split_once(' ')) else { return Parts::everything() };
        // Without renames, so a renamed app's old folder is in it too.
        let (Ok(files), Ok(renames)) = (git(src, &["diff", "--name-status", "--no-renames", was, now]), git(src, &["diff", "--name-status", "-M", "--diff-filter=R", was, now, "--", "apps/*/app.js"])) else {
            return Parts::everything();
        };
        let mut parts = Parts::of(files.lines().filter_map(|l| l.split_once('\t')).map(|(_, f)| f));
        // A new built-in app is in the node's next build, which adds it.
        if files.lines().any(|l| l.strip_prefix("A\tapps/").is_some_and(|f| f.ends_with("/app.js") && f.matches('/').count() == 1)) {
            (parts.node, parts.mac) = (true, true);
        }
        let app = |f: &str| f.split('/').nth(1).unwrap_or("").to_string();
        parts.renamed = renames
            .lines()
            .filter_map(|l| {
                let mut f = l.split('\t').skip(1);
                Some((app(f.next()?), app(f.next()?)))
            })
            .collect();
        // The config's change sends everything; the apps' diff still says which folders went and what a rename became.
        if was_config != now_config {
            (parts.node, parts.mac) = (true, true);
        }
        parts
    }

    /// What changing `files` (paths in the repo) has to send out. Anything not known to be outside the node is in it.
    fn of<'a>(files: impl IntoIterator<Item = &'a str>) -> Parts {
        let mut parts = Parts::default();
        for file in files {
            let top = file.split_once('/').map(|(top, _)| top);
            match top {
                Some("apps") => {
                    let app = file.split('/').nth(1).unwrap_or("").to_string();
                    if !parts.apps.contains(&app) {
                        parts.apps.push(app);
                    }
                }
                Some("apple") => parts.mac = true,
                // The kernel's sources and the serverless node's are in the node, which deploys them; the Mac's needn't.
                Some("kernel") if !file.starts_with("kernel/test/") => parts.node = true,
                Some("serverless") => parts.node = true,
                // The client is the Macs', and in the node too: its WebAssembly is the web client's, and the serverless node's.
                Some("client") => (parts.node, parts.mac) = (true, true),
                Some("docs" | "tests" | "android" | "windows" | "sidecars" | "kernel") => {}
                None if file.ends_with(".md") || file == "LICENSE" || file == ".gitignore" => {}
                _ => (parts.node, parts.mac) = (true, true),
            }
        }
        parts
    }
}

/// Runs a command, showing its output only if it fails.
pub fn run_quiet(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("run {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!(
            "{:?} {} failed:\n{}{}",
            cmd.get_program(),
            cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

/// Deletes the built-in apps whose folder is gone from `src`, renamed or removed, where the install still has them:
/// they'd stay as apps no release ships. `built_in` is the bundle the running release ships under a name, which says
/// it whatever the update sends; `deploys` 0 is one it offers but the install never added. They wait in Recently
/// Deleted, where an admin could restore one. Gives each renamed one's new name, with where the old one was installed.
fn retire_gone(src: &Path, renamed: &[(String, String)]) -> Result<Vec<(String, Vec<String>)>> {
    let mut moved = Vec::new();
    for app in get("/v1/apps")?.as_array().into_iter().flatten() {
        let had = app["built_in"].is_string() && app["deploys"].as_u64().is_some_and(|n| n > 0);
        let Some(name) = app["name"].as_str().filter(|_| had) else { continue };
        if app["deleted"].is_null() && !src.join("apps").join(name).join("app.js").is_file() {
            post("/v1/retire", json!({ "app": name, "how": "deleted" }))?;
            match renamed.iter().find(|(old, _)| old == name) {
                Some((_, new)) => {
                    println!("Deleted {name}: it's {new} now");
                    let scopes = app["installed"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
                    moved.push((new.clone(), scopes));
                }
                None => println!("Deleted {name}: its folder is gone from apps/, so no release ships it"),
            }
        }
    }
    Ok(moved)
}

/// Adds each renamed built-in app where its old name was installed.
fn add_where(moved: &[(String, Vec<String>)]) {
    for (app, scopes) in moved {
        for scope in scopes {
            match post("/v1/install", json!({ "app": app, "scope": scope, "installed": true })) {
                Ok(_) => println!("Added {app} to {scope}, where its old name was"),
                Err(err) => println!("  couldn't add {app} to {scope}: {err:#}"),
            }
        }
    }
}

/// Deploys the built-in apps named, as they are in `src`, where the install has them.
fn deploy_apps(src: &Path, names: &[String]) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    for app in get("/v1/apps")?.as_array().into_iter().flatten() {
        let Some(name) = app["name"].as_str().filter(|n| app["built_in"].is_string() && names.iter().any(|m| m == n)) else { continue };
        let dir = src.join("apps").join(name);
        if !dir.join("app.js").is_file() {
            continue;
        }
        let files = one::bundle::sources(&dir)?;
        let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
        let bundle = one::node::built_in_bundle(&files)?;
        let blob = crate::request(crate::agent().post(format!("http://{}/v1/blobs", crate::addr())).send(&bundle[..]))?["blob"].as_str().unwrap_or("").to_string();
        if app["bundle"].as_str() != Some(blob.as_str()) {
            post("/v1/deploy", json!({ "app": name, "bundle": blob }))?;
            println!("Deployed {name}'s new version ({})", &blob[..12]);
        }
    }
    Ok(())
}

/// Has each Mac build `commit`'s app now, instead of anything older it's building, then the iPhone app, for the iPhone it
/// reaches; says what its last builds did. True if a Mac was missed.
fn follow(macs: &[Value], commit: &str, config_commit: Option<&str>) -> bool {
    let mut missed = false;
    for mac in macs {
        let name = mac["name"].as_str().unwrap_or("");
        if mac["online"] != true {
            println!("  {name}: offline; run this again when it's back");
            missed = true;
            continue;
        }
        match shell(mac, &mac_follow(commit, config_commit), 60) {
            Ok(r) if r["exit"] == 0 => {
                // Its report, with logs indented under what they're of.
                for line in r["stdout"].as_str().unwrap_or("").lines() {
                    if line.starts_with(' ') { println!("  {line}") } else { println!("  {name}: {line}") }
                }
            }
            Ok(r) => {
                missed = true;
                println!("  {name}: couldn't start its build: {}", r["stderr"].as_str().unwrap_or("").trim());
            }
            Err(err) => {
                missed = true;
                println!("  {name}: couldn't start its build: {err:#}");
            }
        }
    }
    missed
}

/// This device's turn to update, held until it's dropped: other updates wait for it.
fn turn() -> Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    std::fs::create_dir_all(cache())?;
    let file = std::fs::File::create(cache().join("update.lock"))?;
    // SAFETY: flock on a descriptor this function owns; the lock goes when the file closes.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        println!("Another update is running here; this one goes next, with main's tip then.");
        ensure!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0, "couldn't wait for the other update: {}", std::io::Error::last_os_error());
    }
    Ok(file)
}

/// Deploys the kernel, with the build being sent out (`commit`), from each of `linux` that has a kernel's settings: only
/// there are its secrets (`kernel deploy` in its home's kernel/). It waits for each to run that build, so it never
/// deploys the one before, and tries again what may pass (a link dropping as nodes restart, Cloudflare). Every install
/// has a kernel, so it fails if no device deployed it. True if it failed.
fn deploy_kernel(linux: &[Value], commit: &str) -> bool {
    let home = format!("${{{}_HOME:-${{XDG_DATA_HOME:-$HOME/.local/share}}/{CLI}}}", CLI.to_uppercase());
    // The node's own build deploys it (what's called the CLI there may be something else), once it's the one sent
    // out: 75 says it isn't yet. On another device that's Shell's sh's parent; on this one, which needs no Shell on
    // anyone's pages, the node that wrote its pid. As a login there would: its profile is where 1Password's
    // sign-in, which the kernel's secrets are read with, usually is, and ~/.local/bin where op may be.
    let script = |node: &str| format!(
        r#"[ -f "{home}/kernel/settings.json" ] || {{ echo none; exit 0; }}; node=/proc/{node}/exe
case "$("$node" version --json)" in *'"commit":"{commit}"'*) ;; *) echo "still on $("$node" version --json)"; exit 75 ;; esac
. "$HOME/.profile" >/dev/null 2>&1; PATH="$HOME/.local/bin:$PATH" "$node" kernel deploy"#
    );
    let (here, others): (Vec<&Value>, Vec<&Value>) = linux.iter().filter(|d| d["online"] == true).partition(|d| d["here"] == true);
    let (mut failed, mut deployed) = (false, false);
    // This device first: if its settings are here, nothing is asked of another's Shell.
    for device in here.into_iter().chain(others) {
        if deployed {
            break;
        }
        let name = device["name"].as_str().unwrap_or("");
        let local = device["here"] == true;
        let deploy = script(&if local { format!("$(cat \"{home}/pid\")") } else { "$PPID".into() });
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut failures = 0;
        let said = loop {
            let said = if local { run_here(&deploy) } else { shell(device, &deploy, 600) };
            let again = match &said {
                Err(_) => true,
                Ok(r) if r["exit"] == 75 => true,
                Ok(r) if r["exit"] != 0 => {
                    failures += 1;
                    failures < 3
                }
                Ok(_) => false,
            };
            if !again || Instant::now() > deadline {
                break said;
            }
            std::thread::sleep(Duration::from_secs(10));
        };
        match said {
            Ok(r) if r["exit"] == 0 && r["stdout"].as_str().is_some_and(|o| o.trim() == "none") => {}
            Ok(r) if r["exit"] == 0 => {
                deployed = true;
                println!("  {name}: deployed the kernel at {commit}");
            }
            Ok(r) => {
                failed = true;
                println!("  {name}: couldn't deploy the kernel:\n{}{}", r["stdout"].as_str().unwrap_or(""), r["stderr"].as_str().unwrap_or("").trim());
            }
            Err(err) => {
                failed = true;
                let hint = if format!("{err:#}").contains("isn't on your pages") { " (add Shell in the App Store, or run one update on the device with the kernel's settings)" } else { "" };
                println!("  {name}: couldn't deploy the kernel: {err:#}{hint}");
            }
        }
    }
    if !deployed && !failed {
        println!("  no device deployed the kernel: none of them online has its settings");
    }
    failed || !deployed
}

/// This device's `after-update`, running beside the update that started it: what it said is shown when it's done,
/// however the update ends.
struct After {
    child: Option<std::process::Child>,
    log: PathBuf,
}

impl After {
    /// Waits for it to finish, and shows what it said: → whether it succeeded.
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

/// Waits for `after-update` to finish, and shows what it said.
fn finish(after: Option<After>) -> Result<()> {
    ensure!(after.is_none_or(|mut after| after.wait()), "after-update failed");
    Ok(())
}

/// Deploys this device's release's version of each built-in app the install has, where it runs another.
fn deploy_built_in() -> Result<()> {
    for app in get("/v1/apps")?.as_array().into_iter().flatten() {
        let (Some(name), Some(bundle)) = (app["name"].as_str(), app["built_in"].as_str()) else { continue };
        if app["bundle"].as_str() != Some(bundle) {
            post("/v1/deploy", json!({ "app": name, "bundle": bundle }))?;
            println!("Deployed {name}'s new version ({})", &bundle[..12]);
        }
    }
    Ok(())
}

/// `v1.2.3`, or `v1.2.3-rc.1`.
fn version_tag(tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix('v') else { return false };
    let core = rest.split_once('-').map_or(rest, |(core, pre)| if pre.is_empty() { "" } else { core });
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether install.sh, which gets releases from GitHub, installs this white label. Only that white label is released there.
fn installer_is_ours() -> bool {
    include_str!("../install.sh").contains(&format!("\ncli={CLI}\n"))
}

/// What the Mac app is published as.
fn app_zip() -> String {
    format!("{CLI}-macos.zip")
}

/// The release key's private half, from the publisher's password manager by way of the config's release_signer, kept
/// only in memory. It must be the key every build checks releases against.
pub fn release_key() -> Result<ssh_key::PrivateKey> {
    ensure!(!RELEASE_KEY.is_empty() && !RELEASE_SIGNER.is_empty(), "this white label has no release key, so nothing it builds can be released");
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

/// A clean clone of the repo, kept in the cache, at the tip of main on GitHub. Build output stays, so builds are quick.
fn checkout() -> Result<PathBuf> {
    let dir = cache().join("src");
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(cache())?;
        step(Command::new("git").args(["clone", "-q", &format!("https://github.com/{REPO}.git")]).arg(&dir))?;
    }
    for args in [&["fetch", "-q", "--tags", "--force", "origin"][..], &["checkout", "-q", "--detach", "--force", "origin/main"], &["clean", "-qfdx", "-e", "/target"]] {
        step(Command::new("git").current_dir(&dir).args(args))?;
    }
    Ok(dir)
}

/// The config to build with: this checkout's own, or, for a white label whose config lives in its own repo, that repo's
/// latest, fetched beside it; and then the commit it's at.
fn config(src: &Path) -> Result<(PathBuf, Option<String>)> {
    if CONFIG_REPO.is_empty() {
        let file = src.join(CONFIG_FILE);
        ensure!(file.exists(), "{} isn't there to build with", file.display());
        return Ok((file, None));
    }
    let dir = cache().join("config");
    if !dir.join(".git").exists() {
        step(Command::new("git").args(["clone", "-q", CONFIG_REPO]).arg(&dir))?;
    }
    for args in [&["fetch", "-q", "origin", "HEAD"][..], &["checkout", "-q", "--detach", "--force", "FETCH_HEAD"]] {
        step(Command::new("git").current_dir(&dir).args(args))?;
    }
    Ok((dir.join(CONFIG_FILE), Some(git(&dir, &["rev-parse", "HEAD"])?)))
}

/// Builds the serverless node (`serverless/`) into WebAssembly and its JavaScript glue, for the release to carry: → the
/// folder they're in. It needs Rust's wasm32-unknown-unknown target, a clang that builds for it (SQLite is C), and the
/// wasm-bindgen of the version it's built with.
fn serverless(src: &Path, config: &Path) -> Result<PathBuf> {
    run_quiet(Command::new(cargo()).current_dir(src).env("CONFIG", config).args(["build", "--release", "-p", "one-serverless", "--target", "wasm32-unknown-unknown"]))
        .context("build the serverless node (rustup target add wasm32-unknown-unknown)")?;
    let out = src.join("target/serverless");
    let wasm = src.join("target/wasm32-unknown-unknown/release/one_serverless.wasm");
    run_quiet(Command::new("wasm-bindgen").args(["--target", "web", "--out-dir"]).arg(&out).arg(&wasm)).context("make its glue (cargo install wasm-bindgen-cli, the version in Cargo.lock)")?;
    Ok(out)
}

/// Builds the client (`client/`) into WebAssembly for browsers, as small as it builds, and its JavaScript glue: → the
/// folder they're in. It needs what `serverless` does, and makes it smaller still with wasm-opt where it's installed.
pub fn client(src: &Path, config: &Path) -> Result<PathBuf> {
    run_quiet(Command::new(cargo()).current_dir(src).env("CONFIG", config).args(["rustc", "-p", "one-client", "--profile", "web", "--target", "wasm32-unknown-unknown", "--crate-type", "cdylib"]))
        .context("build the client (rustup target add wasm32-unknown-unknown)")?;
    let out = src.join("target/client");
    let wasm = src.join("target/wasm32-unknown-unknown/web/one_client.wasm");
    let built = out.join("one_client_bg.wasm");
    // Its glue is only made again for a client cargo built again: one that's already here is what a build that has it
    // embedded (`CLIENT`), so rewriting it would have that build done again.
    let modified = |path: &Path| path.metadata().and_then(|m| m.modified()).ok();
    if modified(&built).is_some_and(|glued| modified(&wasm).is_some_and(|built| glued > built)) {
        return Ok(out);
    }
    run_quiet(Command::new("wasm-bindgen").args(["--target", "web", "--out-dir"]).arg(&out).arg(&wasm)).context("make its glue (cargo install wasm-bindgen-cli, the version in Cargo.lock)")?;
    if quiet(Command::new("wasm-opt").arg("--version")) {
        run_quiet(Command::new("wasm-opt").args(["-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int", "--enable-sign-ext", "--enable-mutable-globals", "--enable-reference-types", "--enable-multivalue", "-o"]).arg(&built).arg(&built))?;
    }
    Ok(out)
}

/// Cargo, also where rustup puts it, for a shell that doesn't have it on its PATH.
pub fn cargo() -> PathBuf {
    let rustup = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cargo/bin/cargo");
    if quiet(Command::new("cargo").arg("--version")) || !rustup.exists() { "cargo".into() } else { rustup }
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    ensure!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Runs a command, showing its output.
fn step(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("run {:?}", cmd.get_program()))?;
    ensure!(status.success(), "{:?} {} failed", cmd.get_program(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    Ok(())
}

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// This user's own devices, as this one sees them now: none while its node restarts.
fn devices() -> Vec<Value> {
    let state = get("/v1/state").unwrap_or_default();
    let me = state["install"]["user"].clone();
    state["install"]["devices"].as_array().into_iter().flatten().filter(|d| d["user"] == me).cloned().collect()
}

/// Waits for each of `wanted` to say it runs `commit`, and reports each. True if any didn't.
fn await_builds(wanted: &[Value], commit: &str, since: Instant, limit: Duration) -> bool {
    let id = |d: &Value| d["key"].as_str().unwrap_or("").to_string();
    let name = |d: &Value| d["name"].as_str().unwrap_or("").to_string();
    // Only devices offline from the start count as offline: this device's own node restarting loses its sessions a moment.
    let (online, offline): (Vec<&Value>, Vec<&Value>) = wanted.iter().partition(|d| d["online"] == true);
    for d in offline {
        println!("  {}: offline; it updates itself when it's back", name(d));
    }
    let mut waiting: Vec<(String, String, String)> = online.into_iter().map(|d| (id(d), name(d), d["build"].as_str().unwrap_or("?").into())).collect();
    let deadline = Instant::now() + limit;
    loop {
        let now = devices();
        waiting.retain(|(device, name, was)| {
            let Some(d) = now.iter().find(|d| d["key"] == device.as_str()) else { return true };
            if d["build"] == commit {
                if was == commit {
                    println!("  {name}: already on {commit}");
                } else {
                    println!("  {name}: {was} → {commit} ({}s)", since.elapsed().as_secs());
                }
                return false;
            }
            true
        });
        if waiting.is_empty() {
            return false;
        }
        if Instant::now() > deadline {
            for (_, name, was) in &waiting {
                println!("  {name}: still on {was}");
            }
            return true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// A command in this machine's own sh, answered as `shell` answers: its exit and what it said.
fn run_here(command: &str) -> Result<Value> {
    let out = Command::new("sh").args(["-c", command]).output().context("run sh")?;
    Ok(json!({ "exit": out.status.code().unwrap_or(-1), "stdout": String::from_utf8_lossy(&out.stdout), "stderr": String::from_utf8_lossy(&out.stderr) }))
}

fn shell(device: &Value, command: &str, timeout: u64) -> Result<Value> {
    let r = post("/v1/call", json!({ "device": device["key"], "app": "shell", "op": "run", "input": { "command": command, "timeout": timeout } }))?;
    Ok(r["ok"].clone())
}

/// Shell for a Mac: start a job in its logged-in session (codesign needs the keychain, and the job outlives the node,
/// which the new app replaces) that builds `commit`'s app in a clean clone and installs it, then leaves the app, zipped,
/// and the node binary as blobs, listed in assets.txt, for a release.
fn mac_job(commit: &str, config_commit: Option<&str>) -> String {
    let assets = format!(
        r#"a="$c/assets"; rm -rf "$a"; mkdir "$a"
cp target/release/{bin} "$a/{CLI}-$(rustc -vV | sed -n 's/^host: //p')"
ditto -c -k --keepParent "$HOME/Applications/{NAME}.app" "$a/{zip}"
for f in "$a"/*; do
	n=0; until h=$(target/release/{bin} file put "$f"); do n=$((n + 1)); [ $n -lt 30 ]; sleep 2; done
	echo "$h $(basename "$f")" >> "$c/assets.txt"
done"#,
        zip = app_zip(),
        bin = env!("CARGO_BIN_NAME")
    );
    format!(
        r#"set -e
c="$HOME/Library/Caches/{CLI}"; label=app.{CLI}.update
mkdir -p "$c"; cd "$c"; rm -f update.status assets.txt
cat > update.sh <<'JOB'
c="$HOME/Library/Caches/{CLI}"
exec > "$c/update.log" 2>&1
trap 'echo $? > "$c/update.status"' EXIT
set -e
{checkout}
CONFIG="$config" apple/build.sh --release
{assets}
JOB
{start}"#,
        checkout = mac_checkout(commit, config_commit.unwrap_or("")),
        start = mac_start("update.sh")
    )
}

/// Shell for a Mac: has it build `commit`'s app now, and install it, then the iPhone app, if what that's made of changed,
/// for the iPhone, all in one job in its logged-in session: a newer update stops the build it's on and builds that
/// instead. It installs the iPhone app whenever the phone is reachable, and keeps trying until it is. Says how its last
/// builds failed, once, and what the iPhone has.
fn mac_follow(commit: &str, config_commit: Option<&str>) -> String {
    let short = &commit[..commit.len().min(12)];
    format!(
        r#"set -e
c="$HOME/Library/Caches/{CLI}"; label=app.{CLI}.update
mkdir -p "$c"; cd "$c"
echo "{commit} {config}" > want
if [ -f failed ]; then echo "its build of $(cut -c1-12 failed) failed:"; sed 's/^/    /' failed.log; rm -f failed; fi
if [ -f phone.failed ]; then echo "its iPhone build of $(cut -c1-12 phone.failed) failed:"; sed 's/^/    /' phone.failed.log; rm -f phone.failed; fi
if [ -s phone.built ]; then
	b=$(cut -c1-12 phone.built)
	if [ "$(cat phone.built)" = "$(cat phone.installed 2>/dev/null)" ]; then echo "iPhone: installed $b"; else echo "iPhone: not reachable, will install $b when it is"; fi
fi
echo "builds {short} now, instead of anything older, then the iPhone app if it changed"
pid=$(launchctl list | awk -v l="$label" '$3 == l {{ print $1 }}')
if [ -n "$pid" ] && [ "$pid" != "-" ]; then exit 0; fi
cat > build.sh <<'JOB'
set -e
c="$HOME/Library/Caches/{CLI}"
{checkout}
CONFIG="$config" apple/build.sh
JOB
cat > phone.sh <<'JOB'
set -e
c="$HOME/Library/Caches/{CLI}"
{checkout}
# Only what the iPhone app is made of: the node isn't in it, and each install closes the app on the phone.
last=$(cat "$c/phone.built" 2>/dev/null || true)
if [ "${{last#* }}" = "${{2:-}}" ] && git diff --quiet "${{last%% *}}" "$1" -- apple ':!apple/Mac' vendor/phosphor {CONFIG_FILE} 2>/dev/null; then exit 3; fi
CONFIG="$config" apple/build-phone.sh device "$c/phone"
JOB
cat > follow.sh <<'JOB'
# launchd runs this in the login shell, zsh on a Mac; the job control it uses is bash's.
[ -n "$BASH_VERSION" ] || exec /bin/bash "$0" "$@"
c="$HOME/Library/Caches/{CLI}"
# Each build in a process group of its own, which a newer update stops (apple/build.sh's install ignores that).
set -m
want() {{ cat "$c/want"; }}
# Runs a build at a lower priority, so the Mac stays quick to use, and stops it for a newer update's (sets stopped).
run() {{
	stopped=
	nice -n 10 /bin/sh "$@" & pid=$!
	while kill -0 $pid 2>/dev/null; do
		sleep 2
		if [ "$(want)" != "$want" ]; then stopped=1; kill -TERM -- -$pid 2>/dev/null; break; fi
	done
	wait $pid
}}
# The paired iPhone devicectl reaches, unlocked; the kept iPhone app is installed on it, not opened: if it's in use,
# installing replaces it all the same, so it's left for the person to open.
install() {{
	xcrun devicectl list devices --json-output "$c/devices.json" > /dev/null 2>&1 || return 1
	phone=$(jq -r '[.result.devices[] | select(.hardwareProperties.reality == "physical" and .hardwareProperties.deviceType == "iPhone" and .connectionProperties.pairingState == "paired")][0].identifier // empty' "$c/devices.json")
	[ -n "$phone" ] && xcrun devicectl device install app --device "$phone" "$c"/phone/*.app
}}
while :; do
	want=$(want)
	if [ "$want" != "$(cat "$c/tried" 2>/dev/null)" ]; then
		# zsh doesn't split "$want" into its commit and config: this is bash.
		set -- ${{want%% *}} "${{want#* }}"
		run "$c/build.sh" "$1" "$2" > "$c/update.log" 2>&1; status=$?
		[ -z "$stopped" ] || continue
		if [ $status = 0 ]; then echo "$want" > "$c/built"; else
			tail -n 25 "$c/update.log" > "$c/failed.log"; echo "$want" | tee "$c/failed" > "$c/tried"; continue
		fi
		# 3: nothing the iPhone app is made of changed.
		run "$c/phone.sh" "$1" "$2" > "$c/phone.log" 2>&1; status=$?
		[ -z "$stopped" ] || continue
		case $status in
		0) echo "$want" > "$c/phone.built" ;;
		3) ;;
		*) tail -n 25 "$c/phone.log" > "$c/phone.failed.log"; echo "$want" > "$c/phone.failed" ;;
		esac
		echo "$want" > "$c/tried"
	fi
	[ -s "$c/phone.built" ] && [ "$(cat "$c/phone.built")" != "$(cat "$c/phone.installed" 2>/dev/null)" ] || exit 0
	if install >> "$c/phone.log" 2>&1; then cp "$c/phone.built" "$c/phone.installed"; continue; fi
	# Away or locked: try again in a minute, or build a newer update at once.
	for _ in $(seq 30); do sleep 2; [ "$(want)" = "$want" ] || break; done
done
JOB
{start}"#,
        config = config_commit.unwrap_or(""),
        checkout = mac_checkout("$1", "${2:-}"),
        start = mac_start("follow.sh")
    )
}

/// Shell that puts the Mac's clean clone at `commit` and sets `config` to the config to build with: the clone's own, or
/// for a white label whose config is in its own repo, that repo at `config_commit`.
fn mac_checkout(commit: &str, config_commit: &str) -> String {
    let config = if CONFIG_REPO.is_empty() {
        format!(r#"config="$c/src/{CONFIG_FILE}""#)
    } else {
        format!(
            r#"[ -d "$c/config/.git" ] || git clone -q {CONFIG_REPO} "$c/config"
git -C "$c/config" fetch -q origin
git -C "$c/config" checkout -q --detach --force "{config_commit}"
config="$c/config/{CONFIG_FILE}""#
        )
    };
    format!(
        r#"[ -d "$c/src/.git" ] || git clone -q https://github.com/{REPO}.git "$c/src"
cd "$c/src"
git fetch -q --tags --force origin
git checkout -q --detach --force "{commit}"
git clean -qfdx -e /target -e /apple/build
{config}"#
    )
}

/// Shell that runs `script` (in the Mac's cache) as a job of its logged-in session, from a login shell.
fn mac_start(script: &str) -> String {
    format!(
        r#"cat > update.plist <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>Label</key><string>$label</string><key>RunAtLoad</key><true/>
<key>ProgramArguments</key><array><string>${{SHELL:-/bin/zsh}}</string><string>-l</string><string>$c/{script}</string></array>
</dict></plist>
PLIST
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" update.plist"#
    )
}

/// Waits for a Mac's job to finish; on success, the assets it left as `(blob, name)`, and the warnings its build gave.
fn await_mac(mac: &Value) -> Result<(Vec<(String, String)>, Vec<String>)> {
    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    let poll = format!(r#"c="$HOME/Library/Caches/{CLI}"; cat "$c/update.status" 2>/dev/null"#);
    // The Mac's node restarts on the new app part way through, and doesn't answer for a moment.
    let status = loop {
        ensure!(Instant::now() < deadline, "its build took more than 30 minutes");
        std::thread::sleep(Duration::from_secs(5));
        if let Ok(r) = shell(mac, &poll, 30)
            && let Some(code) = r["stdout"].as_str().and_then(|s| s.trim().parse::<i32>().ok())
        {
            break code;
        }
    };
    let report = shell(mac, &format!(r#"c="$HOME/Library/Caches/{CLI}"; launchctl bootout "gui/$(id -u)/app.{CLI}.update" 2>/dev/null; cat "$c/assets.txt" 2>/dev/null; echo ---; grep '^warning: ' "$c/update.log"; tail -n 25 "$c/update.log""#), 30)?;
    let out = report["stdout"].as_str().unwrap_or("");
    let (made, log) = out.split_once("---\n").unwrap_or(("", out));
    if status != 0 {
        bail!("its build failed:\n{log}");
    }
    let mut warnings: Vec<String> = log.lines().filter(|l| l.starts_with("warning: ")).map(str::to_string).collect();
    warnings.sort();
    warnings.dedup();
    Ok((made.lines().filter_map(|l| l.split_once(' ')).map(|(h, n)| (h.to_string(), n.to_string())).collect(), warnings))
}

fn blob(hash: &str) -> Result<Vec<u8>> {
    let mut resp = crate::agent().get(format!("http://{}/v1/blobs/{hash}", crate::addr())).call()?;
    Ok(resp.body_mut().with_config().limit(1 << 30).read_to_vec()?)
}

#[test]
fn a_change_sends_out_only_what_it_touched() {
    assert_eq!(Parts::of(["README.md", "docs/a.md", "tests/e2e.rs", "android/x", "windows/App.cs", "kernel/test/tests.js"]), Parts::default());
    assert_eq!(Parts::of(["apps/board/app.js", "apps/board/screen.html", "apps/vault/app.js"]), Parts { apps: vec!["board".into(), "vault".into()], ..Default::default() });
    assert_eq!(Parts::of(["apple/Mac/App.swift"]), Parts { mac: true, ..Default::default() });
    assert_eq!(Parts::of(["src/node.rs", "apps/board/app.js"]), Parts { node: true, mac: true, apps: vec!["board".into()], ..Default::default() });
    assert_eq!(Parts::of(["kernel/signer/worker.js", "serverless/worker.js"]), Parts { node: true, ..Default::default() });
    assert_eq!(Parts::of(["Cargo.lock"]), Parts::everything());
}

/// A config change sends everything, but the apps' diff still names the folders that went: a renamed built-in's old
/// name is retired and its new name goes where the old was.
#[test]
fn a_config_change_still_names_the_apps_that_went() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("one-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let git = |args: &[&str]| {
        let out = Command::new("git").current_dir(&dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let commit = |message: &str| {
        git(&["add", "-A"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", message]);
        git(&["rev-parse", "HEAD"])
    };
    git(&["init", "-q"]);
    std::fs::create_dir_all(dir.join("apps/old"))?;
    std::fs::write(dir.join("apps/old/app.js"), "let x = 1")?;
    let was = commit("one");
    std::fs::rename(dir.join("apps/old"), dir.join("apps/new"))?;
    let now = commit("two");
    let renamed = vec![("old".to_string(), "new".to_string())];
    let same = Parts::since(&dir, &format!("{was} c"), &format!("{now} c"));
    assert_eq!(same.renamed, renamed);
    assert!(same.apps.contains(&"old".to_string()) && same.apps.contains(&"new".to_string()));
    let changed = Parts::since(&dir, &format!("{was} c"), &format!("{now} d"));
    assert!(changed.node && changed.mac, "the config's change sends everything");
    assert_eq!((changed.apps, changed.renamed), (same.apps, same.renamed));
    let only = Parts::since(&dir, &format!("{now} c"), &format!("{now} d"));
    assert_eq!(only, Parts { node: true, mac: true, ..Default::default() });
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
