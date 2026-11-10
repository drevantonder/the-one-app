//! The commands that send a build out: what changed, each part's `update`, a release. What runs anywhere is only ever
//! what's on main.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use one::config::{CLI, CONFIG_FILE, CONFIG_REPO, NAME, RELEASE_KEY, REPO, TARGET};
use one::latest::{Latest, Thing};
use one::release::Version;
use one::{cache, release};
use serde_json::{Value, json};

use one::build::{self, release_key, PLATFORMS, cargo, git, quietly, show, succeeds};

mod apps;
mod changes;
mod cloudflare;
mod services;
mod signoff;
mod update;
mod wait;

use apps::*;
use changes::*;
use cloudflare::*;
use services::*;
use signoff::*;
use update::*;
pub use wait::wait;
use wait::*;

pub fn commands() -> one::cli::Command {
    let form = |usage: &str, about: String| (usage.to_string(), about);
    one::cli::Command {
        help: vec![
            form("changes [<commit>]", "what has to go out for main's tip, or <commit>, part by part, since each part was last sent".into()),
            form("update [workers|web|apps|services|mac|node|dev] [--release]", format!("send out the part named, built and sent on its own, or, with none, each that changed, in order: apps, mac, services, node, workers, web, dev. A part with nothing changed does nothing, and each keeps the commit it sent. --release has a Mac notarize its app. Run it on a device with the release key: {CLI}-dev changes says what's due")),
            form("signoff tested --ran <the tests you chose>", "sign this checkout's pushed commit off as tested: it compiles, its screens type-check and build as committed, and you say which tests you ran. Sending takes only a commit signed off this way".into()),
            form("signoff seen [<commit>] --checked <what you checked>", "sign a commit off as seen working where it went: the last thing a change needs".into()),
            form("ship [<commit>] [--timeout <30m>]", "send out what's tested, as a plain update does, and return when every device online runs it, or say which doesn't and why".into()),
            form("unfinished [--here]", "the changes from the last day that aren't yet tested, sent, running everywhere and seen, or with --here only those this checkout made, and what each needs next".into()),
            form("build --dev", "build this checkout as cargo build does, with the Cloudflare node and web client in it, which a plain cargo build then keeps".into()),
            form("release <vX.Y.Z>", "publish main's tip's build on GitHub for the installer, with the Linux app and the app a Mac published for it".into()),
            form("web bundle [--out <folder>]", "write this build's web client as a bundle for the native apps, ready to sign".into()),
            form("status --until <commit> [--device <name>[,<name>…]] [--timeout <10m>]", "wait until every device online now, or those named (online or not), runs that commit, or a later build, and its services answer, or a build fails; names offline devices and how far behind they are without waiting for them, and fails unless all it waited for made it".into()),
        ],
        run: |g| {
            match g.words().as_slice() {
                ["changes"] => changes(None)?,
                ["changes", commit] => changes(Some(commit))?,
                ["build"] if g.has("dev") => crate::throwaway::build_dev()?,
                ["signoff", "tested"] => {
                    g.takes_only(&["ran"])?;
                    tested(g.flag("ran").context("say which tests you ran: --ran \"contracts auth\", or --ran none if the change needs none")?)?
                }
                ["signoff", "seen"] | ["signoff", "seen", _] => {
                    g.takes_only(&["checked"])?;
                    seen(g.words().get(2).copied(), g.flag("checked").context("say what you checked: --checked \"…\"")?)?
                }
                ["ship"] | ["ship", _] => {
                    g.takes_only(&["timeout"])?;
                    ship(g.words().get(1).copied(), wait::limit(g, 30 * 60)?)?
                }
                ["unfinished"] => {
                    g.takes_only(&["here"])?;
                    unfinished(g.has("here"))?
                }
                ["update"] | ["update", _] => {
                    // A flag takes the word after it, so `--release mac` would read as a release of no part.
                    ensure!(g.flag("release").is_none(), "{CLI}-dev update <part> --release: the part comes first");
                    update(g.words().get(1).copied(), g.has("release"))?;
                }
                ["release", tag] => github_release(tag)?,
                ["status"] if g.has("until") => wait(g)?,
                ["web", "bundle"] => println!("{}", web_bundle(Path::new(g.flag("out").unwrap_or(".")))?.display()),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Builds `at` in this device's own clean checkout: → the folder keeping what it made, which `update` and `release`
/// send out: `<cache>/builds/<build>/`, with `one`, `one-dev`, and `config` saying the config's commit.
fn build_release(at: &At) -> Result<PathBuf> {
    ensure!(!REPO.is_empty(), "this custom build names no repo, so it has nothing to build from");
    ensure!(cfg!(target_os = "linux"), "run this on one of your Linux devices");
    let (src, config) = (&at.src, &at.config_file);
    // Every release carries the Cloudflare node: one without it couldn't deploy to Cloudflare.
    let cloudflare = timed("built the Cloudflare node", || build::cloudflare_node(src, config, "release")).context("couldn't build the Cloudflare node")?;
    // And the web client's way to nodes: one without it couldn't reach them.
    let onet = timed("built the web client's onet", || build::onet(src, config)).context("couldn't build the web client's WebAssembly")?;
    let release = || {
        let mut command = cargo();
        command.current_dir(src).env("CONFIG", config).env("CLOUDFLARE_NODE", &cloudflare).env("ONET", &onet).args(["build", "--release"]);
        command
    };
    timed("built the node", || quietly(&mut release()))?;
    let binary = src.join("target/release/one");
    let version = Version::of(&binary).context("ask the new build what it is")?;
    // Windows' too, cross-built here. A device that can't only leaves Windows behind: Linux and Macs go out regardless.
    let windows = timed("built Windows' program", || quietly(release().args(["--target", build::WINDOWS, "--bin", "one"])))
        .map(|()| src.join("target").join(build::WINDOWS).join("release/one.exe"))
        .inspect_err(|err| println!("  couldn't build for Windows (rustup target add {}, and mingw-w64): {err:#}", build::WINDOWS))
        .ok();
    let builds = cache().join("builds");
    // A day is longer than any send-out takes; each is the size of two programs.
    for old in std::fs::read_dir(&builds).into_iter().flatten().flatten() {
        if old.metadata().and_then(|m| m.modified()).ok().and_then(|at| at.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(24 * 60 * 60)) {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    let out = builds.join(&version.commit);
    std::fs::create_dir_all(&out)?;
    // Beside it and moved into place, so a program running from this folder isn't written over.
    for (made, name) in [(binary, "one"), (src.join("target/release/one-dev"), "one-dev")].into_iter().chain(windows.map(|w| (w, "one.exe"))) {
        std::fs::copy(&made, out.join(format!("{name}.new")))?;
        std::fs::rename(out.join(format!("{name}.new")), out.join(name))?;
    }
    std::fs::write(out.join("config"), at.config_commit())?;
    Ok(out)
}

/// Says what `commit`, or main's tip, has to send out, part by part, since each was last sent.
fn changes(commit: Option<&str>) -> Result<()> {
    let _turn = turn()?;
    let at = match commit {
        Some(_) => At::at(commit)?,
        None => to_send()?,
    };
    println!("{}, against what each part last sent:", at.build_name());
    let yes = |b: bool| if b { "yes".to_string() } else { "no".to_string() };
    let list = |names: Vec<String>| if names.is_empty() { "none".to_string() } else { names.join(", ") };
    for part in ORDER {
        let parts = due(part, &at);
        let said = match part {
            Part::Apps => {
                let renamed = parts.renamed.iter().map(|(old, new)| format!("{old}={new}"));
                let names: Vec<String> = parts.apps.iter().cloned().chain(renamed).collect();
                if parts.all { "all".to_string() } else { list(names) }
            }
            Part::Mac => yes(parts.mac),
            Part::Services => list(services_due(&at, &parts)),
            Part::Node => yes(parts.node),
            Part::Workers => yes(parts.workers),
            Part::Web => yes(parts.web),
            Part::Dev => yes(parts.dev),
        };
        let since = part.last_sent().map_or("nothing sent yet".to_string(), |last| last[..12.min(last.len())].to_string());
        println!("  {}: {said} (since {since})", part.name());
    }
    Ok(())
}

/// Publishes a build on GitHub, for the installer.
fn github_release(tag: &str) -> Result<()> {
    ensure!(version_tag(tag), "a release is named like v1.2.3");
    ensure!(one::state::installer_is_ours(), "install.sh installs another custom build, so {NAME} isn't released on GitHub");
    ensure!(succeeds(Command::new("gh").args(["auth", "status"])), "sign in to GitHub first: gh auth login");
    let key = release_key()?;
    let _turn = turn()?;
    let at = to_send()?;
    let folder = match at.build() {
        Some(folder) => folder,
        None => build_release(&at)?,
    };
    let binary = std::fs::canonicalize(folder.join("one"))?;
    let (src, commit, config) = (at.src.clone(), at.commit.clone(), at.config_file.clone());
    ensure!(!succeeds(Command::new("git").current_dir(&src).args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])), "{tag} already exists");
    let mut failed = false;
    let mut assets = Vec::new();
    let dir = cache().join("release").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let name = format!("{CLI}-{TARGET}");
    std::fs::copy(&binary, dir.join(&name))?;
    assets.push(name);
    // state::install_line() fetches it from the latest release.
    std::fs::write(dir.join("install.sh"), include_str!("../../install.sh"))?;
    assets.push("install.sh".into());
    std::fs::write(dir.join("install.ps1"), include_str!("../../install.ps1"))?;
    assets.push("install.ps1".into());
    if let Some(windows) = PLATFORMS.iter().find(|p| p.target == build::WINDOWS)
        && folder.join("one.exe").is_file()
    {
        std::fs::copy(folder.join("one.exe"), dir.join(windows.asset()))?;
        assets.push(windows.asset());
    }
    // The native app for this platform (client/native, its own workspace), which install.sh puts on a desktop.
    if cfg!(target_os = "linux") {
        println!("Building the Linux app");
        let native = src.join("client/native");
        match quietly(cargo().current_dir(&native).env("CONFIG", &config).args(["build", "--release"])) {
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
        Ok(None) => println!("No Mac published the app for {}, so {tag} has none.", &commit[..12]),
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
    ensure!(!failed, "released {tag}, without the Linux app: see above");
    Ok(())
}


/// This device's turn with its build checkout, held until it's dropped: others wait for it.
fn turn() -> Result<std::fs::File> {
    let asked = Instant::now();
    let file = turn_waiting()?;
    if asked.elapsed() >= Duration::from_secs(3) {
        println!("  waited {}s for the build checkout", asked.elapsed().as_secs());
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
        println!("Another command is using this device's build checkout: waiting for it to end.");
        ensure!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0, "couldn't wait for the build checkout: {}", std::io::Error::last_os_error());
    }
    Ok(file)
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

/// Says in the record that this commit is the one to build: every Mac takes the app published for it, or builds it itself
/// if none is. A release's asks for its app notarized, which is every release's.
fn publish_source(at: &At, release: bool) -> Result<()> {
    let key = release_key()?;
    let version = Version { commit: at.commit.clone(), target: "-".into(), committed: at.committed };
    let mut things: Vec<(String, Option<PathBuf>, Version, String)> = vec![("source".into(), None, version.clone(), at.config_commit().into())];
    if release {
        things.push(("source-release".into(), None, version, at.config_commit().into()));
    }
    build::publish_latest(&crate::node(), &key, &things)?;
    println!("Every Mac's app is now made from {}.", &at.commit[..12]);
    Ok(())
}

/// A clean clone of the repo, kept in the cache, at `commit` or the tip of the repo's main. Build output stays, so builds are quick.
fn checkout(commit: Option<&str>) -> Result<PathBuf> {
    ensure!(cache().is_absolute(), "give {} as a full path", one::config::env("HOME"));
    std::fs::create_dir_all(cache())?;
    let dir = cache().join("src");
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(cache())?;
        show(Command::new("git").args(["clone", "-q", REPO]).arg(&dir))?;
    }
    for args in [&["remote", "set-url", "origin", REPO][..], &["fetch", "-q", "--tags", "--force", "origin"], &["checkout", "-q", "--detach", "--force", commit.map_or("origin/main", |c| c.split('-').next().unwrap_or(c))], &["clean", "-qfdx", "-e", "/target"]] {
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
    let r = crate::node().post("/api/call", json!({ "device": device["name"], "service": "shell", "app": "shell", "function": "run", "input": { "command": command, "timeout": timeout } }))?;
    Ok(r["ok"].clone())
}

/// This build's web client written to `dir` as `<commit>.bundle`, not yet signed: gzip, for `publish web` to sign and publish.
fn web_bundle(dir: &Path) -> Result<PathBuf> {
    let bundle = one::web::bundle::Bundle::of_this_build(one::config::BUILD, one::config::COMMITTED);
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.bundle", one::config::BUILD));
    std::fs::write(&path, bundle.packed()?)?;
    Ok(path)
}

/// The web client of the build in `folder`, signed with the release key and named in the record the native apps read,
/// unless it's what was last published from here.
fn publish_web(folder: &Path, key: &ssh_key::PrivateKey) -> Result<()> {
    let version = Version::of(&folder.join("one")).context("that isn't a build folder")?;
    let config = std::fs::read_to_string(folder.join("config")).context("that isn't a build folder")?;
    let dir = cache().join("web");
    let _ = std::fs::remove_dir_all(&dir);
    // The one-dev built beside it, from the same build of the library, holds the same web client.
    let out = Command::new(folder.join("one-dev")).args(["web", "bundle", "--out"]).arg(&dir).output().context("run the build")?;
    ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    let bundle = one::web::bundle::Bundle::unpacked(&std::fs::read(dir.join(format!("{}.bundle", version.commit)))?)?;
    publish_web_bundle(&bundle, &version, config.trim(), key)
}

/// `bundle` signed with the release key and named in the record the native apps read, unless its files are what was last
/// published from here: an update that didn't change it costs a phone nothing.
fn publish_web_bundle(bundle: &one::web::bundle::Bundle, version: &Version, config: &str, key: &ssh_key::PrivateKey) -> Result<()> {
    let hash = bundle.files_hash();
    let last = cache().join("published-web");
    if std::fs::read_to_string(&last).is_ok_and(|was| was == hash) {
        println!("The web client hasn't changed.");
        return Ok(());
    }
    let dir = cache().join("web");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{}.signed", version.commit));
    std::fs::write(&file, one::web::bundle::signed(key, &bundle.packed()?)?)?;
    build::publish_latest(&crate::node(), key, &[(Thing::WEB.into(), Some(file), version.clone(), config.to_string())])?;
    std::fs::write(&last, hash)?;
    println!("Published the web client of {}.", version.commit);
    Ok(())
}
