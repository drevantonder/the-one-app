//! The commands that send a build out, each one step: what changed, build, publish, deploy, release. Which to run, and
//! when, is `dev/skills/`. What runs anywhere is only ever what's on main.

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
mod wait;

use apps::*;
use changes::*;
use cloudflare::*;
use services::*;
pub use wait::wait;
use wait::*;

pub fn commands() -> one::cli::Command {
    let form = |usage: &str, about: String| (usage.to_string(), about);
    one::cli::Command {
        help: vec![
            form("changes [<commit>]", "what has to go out for main's tip, or <commit>, since what was marked sent: the node, the Mac app, apps, services, this program".into()),
            form("changes --sent [<commit>]", "mark main's tip, or <commit>, as sent, so the next changes starts from it".into()),
            form("build [<commit>] [--config <config.toml>]", "build main's tip, or <commit>, in this device's own clean checkout: the node, with the Cloudflare node and the web client in it, and this program. Prints the folder holding them. The config defaults to this custom build's latest".into()),
            form("build --dev", "build this checkout as cargo build does, with the Cloudflare node and web client in it, which a plain cargo build then keeps".into()),
            form("publish [<binary>] [--service <name>] [--target <triple> --commit <hash> --committed <seconds>]", "sign a program, by default the one beside this, and publish it: every device on its platform takes it".into()),
            form("publish web <build folder>", "sign the web client of a build and publish it for the native apps".into()),
            form("publish source [<commit>] [--release]", "say which commit every Mac's app is made from: a Mac takes the app published for it, or builds it. --release has it notarized".into()),
            form("publish services <name>… [--commit <commit>]", "build these apps' services at main's tip, or <commit>, for every platform, and publish them".into()),
            form("retire [<old>=<new>…]", "delete the built-in apps this device's build ships whose folder is gone from main: run it before publishing the build without them. Says where each renamed one was added".into()),
            form("deploy apps [<checkout>]", "deploy the built-in apps as <checkout> has them, or as this device's build ships them, where the network runs another version".into()),
            form("deploy workers <commit>", "deploy the Workers and the Cloudflare node from their home device, once it runs <commit>: only there are their secrets".into()),
            form("release <vX.Y.Z> <build folder>", "publish a build on GitHub for the installer, with the Linux app and the app a Mac published for it".into()),
            form("web bundle [--out <folder>]", "write this build's web client as a bundle for the native apps, ready to sign".into()),
            form("mac-job", format!("the shell that sets up a Mac to take its own updates: {CLI}-dev mac-job | ssh <mac> sh, once, for a Mac too old to have the job its app starts")),
            form("status --until <commit> [--device <name>[,<name>…]] [--timeout <10m>]", "wait until every device online now, or those named (online or not), runs that commit, or a later build, and its services answer, or a build fails; names offline devices and how far behind they are without waiting for them, and fails unless all it waited for made it".into()),
        ],
        run: |g| {
            match g.words().as_slice() {
                ["changes"] => changes(g, None)?,
                ["changes", commit] => changes(g, Some(commit))?,
                ["build"] if g.has("dev") => crate::throwaway::build_dev()?,
                ["build"] => build_release(None, g.flag("config"))?,
                ["build", commit] => build_release(Some(commit), g.flag("config"))?,
                ["publish", "web", folder] => publish_web(Path::new(folder), &release_key()?)?,
                ["publish", "source"] => publish_source(None, g.has("release"))?,
                ["publish", "source", commit] => publish_source(Some(commit), g.has("release"))?,
                ["publish", "services", names @ ..] if !names.is_empty() => publish_services_at(names, g.flag("commit"))?,
                ["publish", rest @ ..] if rest.len() <= 1 => {
                    let path = match rest {
                        [path] => std::fs::canonicalize(path)?,
                        _ => crate::program()?,
                    };
                    publish(g, &path, &release_key()?)?;
                }
                ["retire", renames @ ..] => retire(renames)?,
                ["deploy", "apps"] => deploy_built_in()?,
                ["deploy", "apps", checkout] => deploy_apps(Path::new(checkout))?,
                ["deploy", "workers", commit] => ensure!(!deploy_cloudflare(commit), "it isn't deployed to Cloudflare"),
                ["release", tag, folder] => github_release(tag, Path::new(folder))?,
                ["status"] if g.has("until") => wait(g)?,
                ["web", "bundle"] => println!("{}", web_bundle(Path::new(g.flag("out").unwrap_or(".")))?.display()),
                ["mac-job"] => print!("{}", one::latest::mac_job(&crate::node().state()?.address().context("this network has no address yet")?)),
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Builds `commit`, or main's tip, in this device's own clean checkout, and prints the folder keeping what it made, which
/// `publish` and `release` take: `<cache>/builds/<commit>/`, with `one`, `one-dev`, and `config` saying the config's commit.
fn build_release(commit: Option<&str>, given: Option<&str>) -> Result<()> {
    ensure!(!REPO.is_empty(), "this custom build names no repo, so it has nothing to build from");
    ensure!(cfg!(target_os = "linux"), "run this on one of your Linux devices");
    let given = given.map(std::fs::canonicalize).transpose().context("no such config")?;
    let _turn = turn()?;
    let src = timed("fetched the code", || checkout(commit))?;
    let (config, config_commit) = match given {
        Some(file) => (file, None),
        None => config(&src)?,
    };
    let mut node = cargo();
    node.current_dir(&src).env("CONFIG", &config).args(["build", "--release"]);
    // Every release carries the Cloudflare node: one without it couldn't deploy to Cloudflare.
    node.env("CLOUDFLARE_NODE", timed("built the Cloudflare node", || build::cloudflare_node(&src, &config, "release")).context("couldn't build the Cloudflare node")?);
    // And the web client's way to nodes: one without it couldn't reach them.
    node.env("CLIENT", timed("built the web client", || build::client(&src, &config)).context("couldn't build the web client's WebAssembly")?);
    timed("built the node", || quietly(&mut node))?;
    let binary = src.join("target/release/one");
    let version = Version::of(&binary).context("ask the new build what it is")?;
    let builds = cache().join("builds");
    // A day is longer than any send-out takes; each is the size of two programs.
    for old in std::fs::read_dir(&builds).into_iter().flatten().flatten() {
        if old.metadata().and_then(|m| m.modified()).ok().and_then(|at| at.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(24 * 60 * 60)) {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    let out = builds.join(&version.commit);
    std::fs::create_dir_all(&out)?;
    std::fs::copy(&binary, out.join("one"))?;
    std::fs::copy(src.join("target/release/one-dev"), out.join("one-dev"))?;
    std::fs::write(out.join("config"), config_commit.as_deref().unwrap_or("-"))?;
    println!("{}", out.display());
    Ok(())
}

/// Says what `commit`, or main's tip, has to send out since what was marked sent here, or marks it sent.
fn changes(g: &one::cli::Given, commit: Option<&str>) -> Result<()> {
    let _turn = turn()?;
    let src = checkout(commit)?;
    let (_, config_commit) = config(&src)?;
    let commit = git(&src, &["rev-parse", "HEAD"])?;
    let this = format!("{commit} {}", config_commit.as_deref().unwrap_or(""));
    let done = cache().join("updated");
    if g.has("sent") {
        std::fs::write(&done, &this)?;
        println!("Marked {} sent.", &commit[..12]);
        return Ok(());
    }
    let last = std::fs::read_to_string(&done).ok();
    let released = services_released();
    let mut parts = last.as_deref().map_or_else(Parts::everything, |last| Parts::since(&src, last, &this));
    for name in services(&src) {
        let rust = service_dir(&src, &name).join("Cargo.toml").is_file();
        if (last.is_none() || !released.contains_key(&name) || (rust && parts.service)) && !parts.services.contains(&name) {
            parts.services.push(name);
        }
    }
    parts.services.retain(|n| services(&src).contains(n));
    let list = |names: &[String]| if names.is_empty() { "none".to_string() } else { names.join(", ") };
    let yes = |b: bool| if b { "yes" } else { "no" };
    let renamed: Vec<String> = parts.renamed.iter().map(|(old, new)| format!("{old}={new}")).collect();
    println!("{} since {}:", &commit[..12], last.as_deref().map_or("nothing sent yet", |l| &l[..12.min(l.len())]));
    println!("  node: {}\n  mac: {}\n  apps: {}\n  renamed apps: {}\n  services: {}\n  {CLI}-dev: {}", yes(parts.node), yes(parts.mac), list(&parts.apps), list(&renamed), list(&parts.services), yes(parts.dev));
    Ok(())
}

/// Builds and publishes the services named, at `commit` or main's tip.
fn publish_services_at(names: &[&str], commit: Option<&str>) -> Result<()> {
    let _turn = turn()?;
    let src = checkout(commit)?;
    let commit = git(&src, &["rev-parse", "HEAD"])?;
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    ensure!(publish_services(&src, &names, &release_key()?, &commit), "not every service was published");
    Ok(())
}

/// Deletes the built-in apps whose folder is gone from main.
fn retire(renames: &[&str]) -> Result<()> {
    let renamed: Vec<(String, String)> = renames.iter().map(|r| r.split_once('=').map(|(o, n)| (o.to_string(), n.to_string())).with_context(|| format!("{r} isn't <old>=<new>"))).collect::<Result<_>>()?;
    let _turn = turn()?;
    retire_gone(&checkout(None)?, &renamed)
}

/// Publishes a build on GitHub, for the installer.
fn github_release(tag: &str, folder: &Path) -> Result<()> {
    ensure!(version_tag(tag), "a release is named like v1.2.3");
    ensure!(build::installer_is_ours(), "install.sh installs another custom build, so {NAME} isn't released on GitHub");
    ensure!(succeeds(Command::new("gh").args(["auth", "status"])), "sign in to GitHub first: gh auth login");
    let binary = std::fs::canonicalize(folder.join("one")).context("that isn't a build folder")?;
    let version = Version::of(&binary).context("ask the build what it is")?;
    let key = release_key()?;
    let _turn = turn()?;
    let src = checkout(Some(&version.commit))?;
    let commit = git(&src, &["rev-parse", "HEAD"])?;
    let (config, _) = config(&src)?;
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
fn publish_source(commit: Option<&str>, release: bool) -> Result<()> {
    let key = release_key()?;
    let _turn = turn()?;
    let src = checkout(commit)?;
    let (_, config_commit) = config(&src)?;
    let config = config_commit.as_deref().unwrap_or("-");
    let commit = git(&src, &["rev-parse", "HEAD"])?;
    let committed = git(&src, &["log", "-1", "--format=%ct", &commit])?.trim().parse().context("the commit's date")?;
    let version = Version { commit: commit.clone(), target: "-".into(), committed };
    let mut things: Vec<(String, Option<PathBuf>, Version, String)> = vec![("source".into(), None, version.clone(), config.into())];
    if release {
        things.push(("source-release".into(), None, version, config.into()));
    }
    build::publish_latest(&crate::node(), &key, &things)?;
    println!("Every Mac's app is now made from {}.", &commit[..12]);
    Ok(())
}

/// A clean clone of the repo, kept in the cache, at `commit` or the tip of main on GitHub. Build output stays, so builds are quick.
fn checkout(commit: Option<&str>) -> Result<PathBuf> {
    ensure!(cache().is_absolute(), "give {} as a full path", one::config::env("HOME"));
    std::fs::create_dir_all(cache())?;
    let dir = cache().join("src");
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(cache())?;
        show(Command::new("git").args(["clone", "-q", &format!("https://github.com/{REPO}.git")]).arg(&dir))?;
    }
    for args in [&["fetch", "-q", "--tags", "--force", "origin"][..], &["checkout", "-q", "--detach", "--force", commit.map_or("origin/main", |c| c.split('-').next().unwrap_or(c))], &["clean", "-qfdx", "-e", "/target"]] {
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

/// This build's web client written to `dir` as `<commit>.bundle`, not yet signed: gzip, for `publish web` to sign and publish.
fn web_bundle(dir: &Path) -> Result<PathBuf> {
    let bundle = one::web::bundle::Bundle::of_this_build(one::config::BUILD, one::config::COMMITTED);
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.bundle", one::config::BUILD));
    std::fs::write(&path, bundle.packed()?)?;
    Ok(path)
}

/// The web client of the build in `folder`, signed with the release key and named in the record the native apps read,
/// unless it's what was last published from here: an update that didn't change it costs a phone nothing.
fn publish_web(folder: &Path, key: &ssh_key::PrivateKey) -> Result<()> {
    let version = Version::of(&folder.join("one")).context("that isn't a build folder")?;
    let config = std::fs::read_to_string(folder.join("config")).context("that isn't a build folder")?;
    let dir = cache().join("web");
    let _ = std::fs::remove_dir_all(&dir);
    // The one-dev built beside it, from the same build of the library, holds the same web client.
    let out = Command::new(folder.join("one-dev")).args(["web", "bundle", "--out"]).arg(&dir).output().context("run the build")?;
    ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    let packed = std::fs::read(dir.join(format!("{}.bundle", version.commit)))?;
    let last = cache().join("published-web");
    if std::fs::read(&last).is_ok_and(|was| was == packed) {
        println!("The web client hasn't changed.");
        return Ok(());
    }
    let file = dir.join(format!("{}.signed", version.commit));
    std::fs::write(&file, one::web::bundle::signed(key, &packed)?)?;
    build::publish_latest(&crate::node(), key, &[("web".into(), Some(file), version.clone(), config.trim().to_string())])?;
    std::fs::write(&last, packed)?;
    println!("Published the web client of {}.", version.commit);
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
