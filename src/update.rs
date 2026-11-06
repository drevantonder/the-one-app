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
/// takes, and which deploys the Workers on Cloudflare (and the Cloudflare node with it) from the device they're deployed from (the first to deploy them: no other does). Beside it runs `after-update` in this
/// device's home, if there is one: what else is built from main here, like another custom build's update.
pub fn run(tag: Option<&str>) -> Result<()> {
    ensure!(!REPO.is_empty(), "this custom build names no repo, so it has nothing to update from");
    ensure!(cfg!(target_os = "linux"), "run this on one of your Linux devices");
    // It runs from its own cache, not the folder it was started in, which (an agent's worktree) may be gone before it ends.
    ensure!(cache().is_absolute(), "give {}_HOME as a full path", CLI.to_uppercase());
    std::fs::create_dir_all(cache())?;
    std::env::set_current_dir(cache())?;
    if let Some(tag) = tag {
        ensure!(version_tag(tag), "a release is named like v1.2.3");
        ensure!(installer_is_ours(), "install.sh installs another custom build, so {NAME} isn't released on GitHub; {CLI} update runs main on its devices");
        ensure!(quiet(Command::new("gh").args(["auth", "status"])), "sign in to GitHub first: gh auth login");
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
    // A service never sent out from here goes out all the same.
    let released = services_released();
    let unreleased = services(&src).iter().any(|n| !released.contains_key(n));
    if tag.is_none() && last.as_deref() == Some(this.as_str()) && mac_last.as_deref() == Some(this.as_str()) && !unreleased {
        println!("Every device already runs {short}, main's tip: the last update here sent it out.");
        return Ok(());
    }
    if let Some(tag) = tag {
        ensure!(!quiet(Command::new("git").current_dir(&src).args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])), "{tag} already exists");
    }
    let mut parts = last.as_deref().map_or_else(Parts::everything, |last| Parts::since(&src, last, &this));
    parts.mac = mac_last.as_deref().map_or_else(Parts::everything, |last| Parts::since(&src, last, &this)).mac;
    // A release sends all of it; the diff still says which apps' folders went and what a rename became.
    if tag.is_some() {
        (parts.node, parts.mac) = (true, true);
    }
    // A service's program is sent out when its folder changed, or it never was from here.
    for name in services(&src) {
        let rust = service_dir(&src, &name).join("Cargo.toml").is_file();
        if (tag.is_some() || last.is_none() || !released.contains_key(&name) || (rust && parts.service)) && !parts.services.contains(&name) {
            parts.services.push(name);
        }
    }
    parts.services.retain(|n| services(&src).contains(n));
    let macs: Vec<Value> =
        if parts.mac { devices().into_iter().filter(|d| d["target"].as_str().is_some_and(|t| t.ends_with("-apple-darwin"))).collect() } else { Vec::new() };

    let sends = parts.node || parts.mac || !parts.apps.is_empty() || !parts.services.is_empty();
    let after = if sends && tag.is_none() { after_update()? } else { None };
    // The Macs start first, since their builds take longest: an update's in the background, and a release's with what it
    // publishes, which it waits for.
    let (mut missed, mut missed_macs) = (false, false);
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
        missed_macs = follow(&macs, &commit, config_commit.as_deref());
    }
    let mark_macs = |missed_macs: bool| {
        if !missed_macs {
            let _ = std::fs::write(&mac_done, &this);
        }
    };
    let moved = retire_gone(&src, &parts.renamed)?;
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
    let mut build = Command::new(cargo());
    build.current_dir(&src).env("CONFIG", &config).args(["build", "--release"]);
    // Every release carries the Cloudflare node: one without it couldn't deploy to Cloudflare.
    build.env("CLOUDFLARE_NODE", timed("built the Cloudflare node", || cloudflare_node(&src, &config, "release")).context("couldn't build the Cloudflare node, so nothing was sent out")?);
    // And the web client's way to nodes: one without it couldn't reach them.
    build.env("CLIENT", timed("built the web client", || client(&src, &config)).context("couldn't build the web client's WebAssembly, so nothing was sent out")?);
    timed("built the node", || run_quiet(&mut build))?;

    let binary = src.join("target/release").join(env!("CARGO_BIN_NAME"));
    // The build devices say they run: main's commit, and a custom build's config's.
    let version: Value = serde_json::from_slice(&Command::new(&binary).args(["version", "--json"]).output()?.stdout).context("ask the new build what it is")?;
    let short = version["commit"].as_str().context("the new build doesn't say what it is")?.to_string();
    let short = short.as_str();
    // Which devices to wait for, asked before this device's own node restarts on the new build.
    let linux: Vec<Value> = devices().into_iter().filter(|d| d["target"] == TARGET).collect();
    // Services first, so a node that no longer carries one (it became its app's own program) finds it out.
    let mut failed = !publish_services(&src, &parts.services, &key, &commit);
    if !linux.is_empty() && linux.iter().all(|d| d["build"] == short) {
        println!("  every Linux device already runs {short}");
    } else {
        timed("published the node", || crate::publish(&binary, &Default::default(), &key))?;
    }
    let started_at = Instant::now();
    failed |= timed("waited for the Linux devices", || await_builds(&linux, short, started_at, Duration::from_secs(180)));
    if let Err(err) = timed("deployed the built-in apps", deploy_built_in) {
        failed = true;
        println!("  couldn't deploy the new versions of the built-in apps: {err:#}");
    }
    add_where(&moved);
    let cloudflare_failed = timed("deployed to Cloudflare", || deploy_cloudflare(&linux, short));

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
        match run_quiet(Command::new(cargo()).current_dir(&native).env("CONFIG", &config).args(["build", "--release"])) {
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
    ensure!(!cloudflare_failed, "released {tag}, but it isn't deployed to Cloudflare");
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
    /// Apps' services (`apps/<name>/service/`), each built for every platform and published.
    services: Vec<String>,
    /// The `service` crate, which every service in Rust is built on, so each is published again.
    service: bool,
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
                // An app's service is built apart from its app (`service_dir`), which its deploy leaves out.
                Some("apps") if file.split('/').nth(2) == Some("service") && file.matches('/').count() >= 3 => {
                    let app = file.split('/').nth(1).unwrap_or("").to_string();
                    if !parts.services.contains(&app) {
                        parts.services.push(app);
                    }
                }
                Some("apps") => {
                    let app = file.split('/').nth(1).unwrap_or("").to_string();
                    if !parts.apps.contains(&app) {
                        parts.apps.push(app);
                    }
                }
                Some("service") => parts.service = true,
                // The Workers' sources and the Cloudflare node's are in the node, which deploys them; the Mac's needn't.
                Some("cloudflare") if !file.starts_with("cloudflare/test/") => parts.node = true,
                // The native app (client/native) is its own workspace, not in the node: the Mac app, but for Windows' and
                // Android's own (client/native/windows/, client/native/android/), which no update sends out.
                Some("client") if file.starts_with("client/native/windows/") || file.starts_with("client/native/android/") => {}
                Some("client") if file.starts_with("client/native/") => parts.mac = true,
                // The client is the Macs', and in the node too: its WebAssembly is the web client's, and the Cloudflare node's.
                Some("client") => (parts.node, parts.mac) = (true, true),
                Some("docs" | "tests" | "cloudflare") => {}
                None if file.ends_with(".md") || file == "LICENSE" || file == ".gitignore" => {}
                _ => (parts.node, parts.mac) = (true, true),
            }
        }
        parts
    }
}

/// The app `name`'s service's source in `src`.
fn service_dir(src: &Path, name: &str) -> PathBuf {
    src.join("apps").join(name).join("service")
}

/// The apps in `src` with a service: each `apps/<name>/service/` with a Go module or a Rust package.
fn services(src: &Path) -> Vec<String> {
    let has = |e: &std::fs::DirEntry| ["go.mod", "Cargo.toml"].iter().any(|f| e.path().join("service").join(f).is_file());
    let mut names: Vec<String> = std::fs::read_dir(src.join("apps")).into_iter().flatten().flatten().filter(has).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    names
}

/// Builds the Go service in `dir` for `os` and `arch` (Go's names) at `out`, saying it was built from `built`.
fn go_build(dir: &Path, os: &str, arch: &str, built: &str, out: &Path) -> Result<()> {
    let mut go = Command::new(go());
    go.current_dir(dir).env("CGO_ENABLED", "0").env("GOOS", os).env("GOARCH", arch);
    go.args(["build", "-trimpath", "-ldflags", &format!("-X main.built={built}"), "-o"]).arg(out).arg(".");
    run_quiet(&mut go)
}

/// A platform as Go names it: its `GOOS` and `GOARCH`.
fn go_platform(target: &str) -> (&'static str, &'static str) {
    match target {
        "x86_64-unknown-linux-gnu" => ("linux", "amd64"),
        "aarch64-unknown-linux-gnu" => ("linux", "arm64"),
        _ => ("darwin", "arm64"),
    }
}

/// A platform as a person says it.
pub fn platform_said(target: &str) -> &'static str {
    match target {
        "x86_64-unknown-linux-gnu" => "Linux",
        "aarch64-unknown-linux-gnu" => "Linux on ARM",
        "aarch64-apple-darwin" => "Mac",
        _ => "another platform",
    }
}

/// Builds the service of the app in `dir` (its `service/`) here, as `deploy` does, for each platform this machine can:
/// one in Go for every platform, one in Rust for this machine's. Nodes never build. → each build by platform, and the
/// platforms it couldn't build for; none if the app has no service.
pub fn build_service(dir: &Path) -> Result<Option<(Vec<(String, Vec<u8>)>, Vec<&'static str>)>> {
    let source = dir.join("service");
    let rust = source.join("Cargo.toml").is_file();
    if !rust && !source.join("go.mod").is_file() {
        return Ok(None);
    }
    let built = git(&source, &["rev-parse", "--short=12", "HEAD"]).unwrap_or_default();
    let out = std::env::temp_dir().join(format!("{CLI}-service-{}", std::process::id()));
    let (mut builds, mut missed) = (Vec::new(), Vec::new());
    for target in PLATFORMS {
        if rust && target != TARGET {
            missed.push(target);
            continue;
        }
        let (os, arch) = go_platform(target);
        match rust {
            true => {
                // Cargo says where it put the program it built.
                let o = Command::new(cargo()).env("ONE_BUILD", &built).args(["build", "--release", "--message-format=json-render-diagnostics", "--manifest-path"]).arg(source.join("Cargo.toml")).output()?;
                ensure!(o.status.success(), "its service didn't build:\n{}", String::from_utf8_lossy(&o.stderr));
                let said = String::from_utf8_lossy(&o.stdout);
                let program = said.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|m| m["executable"].as_str().map(PathBuf::from)).last();
                std::fs::copy(program.context("its service's crate builds no program")?, &out)?;
            }
            false => go_build(&source, os, arch, &built, &out).with_context(|| format!("build its service for {} (Go: go.dev/doc/install)", platform_said(target)))?,
        }
        builds.push((target.to_string(), std::fs::read(&out)?));
        let _ = std::fs::remove_file(&out);
    }
    ensure!(!builds.is_empty(), "its service is in Rust, which this machine ({TARGET}) builds for no platform a node runs: deploy it from Linux or a Mac");
    Ok(Some((builds, missed)))
}

/// Builds the Rust service of the app `name` in the checkout `src` for this machine, at `out`, saying it was built
/// from `built`.
fn cargo_build(src: &Path, name: &str, built: &str, out: &Path) -> Result<()> {
    run_quiet(Command::new(cargo()).current_dir(src).env("ONE_BUILD", built).args(["build", "--release", "-p", &format!("{}-{name}", env!("CARGO_BIN_NAME"))]))?;
    std::fs::copy(src.join("target/release").join(format!("{}-{name}", env!("CARGO_BIN_NAME"))), out)?;
    Ok(())
}

/// Puts your checkout's build of the app's service in `dir` (`apps/<name>`) on `device` (here by default, else reached
/// by ssh as its name) and switches it on there, for trying it before it's sent out. oned runs it until a release of it
/// newer than this build is published.
pub fn try_service(dir: &Path, device: Option<&str>) -> Result<()> {
    let dir = std::fs::canonicalize(dir).with_context(|| format!("{} isn't here", dir.display()))?;
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let source = dir.join("service");
    let rust = source.join("Cargo.toml").is_file();
    ensure!(rust || source.join("go.mod").is_file(), "{} has no service (service/go.mod or service/Cargo.toml)", dir.display());
    let on = |args: &str| -> Result<String> {
        let out = match device {
            Some(device) => Command::new("ssh").args([device, args]).stdin(std::process::Stdio::null()).output(),
            None => Command::new("sh").args(["-c", args]).output(),
        }?;
        ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let there = device.unwrap_or("this device");
    on(&format!("test -x ~/.local/bin/{CLI}")).map_err(|_| anyhow::anyhow!("{CLI} isn't on {there}: install it there first"))?;
    let uname = on("uname -sm")?;
    let (os, arch) = uname.split_once(' ').unwrap_or_default();
    let arch = if arch == "x86_64" { "amd64" } else { "arm64" };
    let commit = git(&source, &["rev-parse", "--short=12", "HEAD"])?.trim().to_string();
    let dirty = if quiet(Command::new("git").current_dir(&source).args(["diff", "--quiet", "HEAD", "--", "."])) { "" } else { "+dirty" };
    let out = std::env::temp_dir().join(format!("{CLI}-{name}-{}", std::process::id()));
    match rust {
        true => {
            ensure!(uname == "Linux x86_64" && TARGET == "x86_64-unknown-linux-gnu", "a service in Rust is built here for this machine, so try it on a Linux x86_64 device from one");
            cargo_build(dir.parent().and_then(Path::parent).unwrap_or(&dir), &name, &format!("{commit}{dirty}"), &out)?
        }
        false => go_build(&source, &os.to_lowercase(), arch, &format!("{commit}{dirty}"), &out)?,
    }
    // Put in place in one step, so oned never starts half of it. Here, where oned looks for it (`oned::fetched`: a throwaway
    // network's own, never the real network's); on another device, in its `~/.local/bin`.
    let program = format!("{CLI}-{name}");
    let copied = match device {
        Some(device) => quiet(Command::new("scp").arg("-q").arg(&out).arg(format!("{device}:.local/bin/{program}.new"))),
        None => {
            let new = one::oned::fetched(&name).with_extension("new");
            new.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok()) && std::fs::copy(&out, new).is_ok()
        }
    };
    let _ = std::fs::remove_file(&out);
    ensure!(copied, "couldn't copy it to {there}");
    match device {
        Some(_) => drop(on(&format!("chmod 755 ~/.local/bin/{program}.new && mv ~/.local/bin/{program}.new ~/.local/bin/{program}"))?),
        None => {
            use std::os::unix::fs::PermissionsExt;
            let path = one::oned::fetched(&name);
            std::fs::set_permissions(path.with_extension("new"), std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(path.with_extension("new"), path)?;
        }
    }
    post("/v1/service", json!({ "name": name, "device": device, "on": true }))?;
    println!("{name}'s service {commit}{dirty} is on {there}: its oned starts it; {CLI} services says when it's running.");
    Ok(())
}

/// Each service's program last published from here, and the commit it was built from.
fn services_released() -> serde_json::Map<String, Value> {
    std::fs::read(cache().join("services-released")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Builds each service named for every platform the installer knows, signs it and publishes it, so each device that
/// runs it takes it (and one that's switched on there fetches it). → false if one wasn't.
fn publish_services(src: &Path, names: &[String], key: &ssh_key::PrivateKey, commit: &str) -> bool {
    let mut ok = true;
    let committed: u64 = git(src, &["log", "-1", "--format=%ct", commit]).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
    let mut released = services_released();
    for name in names {
        let dir = service_dir(src, name);
        let rust = dir.join("Cargo.toml").is_file();
        let each = || -> Result<()> {
            // One in Rust is built for this machine's platform: each Mac builds its own with its app, and carries it.
            for target in PLATFORMS.into_iter().filter(|t| !rust || *t == TARGET) {
                let (os, arch) = go_platform(target);
                let out = cache().join("services").join(format!("{name}-{target}"));
                std::fs::create_dir_all(out.parent().unwrap_or(&out))?;
                match rust {
                    true => cargo_build(src, name, &commit[..12], &out),
                    false => go_build(&dir, os, arch, &commit[..12], &out),
                }
                .with_context(|| format!("build it for {target}"))?;
                let signature = release::sign(key, &std::fs::read(&out)?)?;
                let body = json!({ "path": out, "service": name, "commit": commit, "target": target, "committed": committed, "signature": signature });
                post("/v1/service-release", body).with_context(|| format!("publish it for {target}"))?;
            }
            Ok(())
        };
        match each() {
            Ok(()) => {
                println!("Published {name} {}: each device that runs it takes it.", &commit[..12]);
                released.insert(name.clone(), json!(commit));
            }
            Err(err) => {
                ok = false;
                println!("  couldn't send out {name}: {err:#}");
            }
        }
    }
    let _ = std::fs::write(cache().join("services-released"), serde_json::to_vec(&released).unwrap_or_default());
    ok
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

/// Deletes the built-in apps whose folder is gone from `src`, renamed or removed, where the network still has them:
/// they'd stay as apps no release ships. `built_in` is the bundle the running release ships under a name, which says
/// it whatever the update sends; `deploys` 0 is one it offers but the network never added. Only one still running the
/// release's own code goes: one an admin deployed their own code to (from its own repo, say) is theirs now, and stays.
/// They wait in Recently Deleted, where an admin could restore one. Gives each renamed one's new name, with where the old one was installed.
fn retire_gone(src: &Path, renamed: &[(String, String)]) -> Result<Vec<(String, Vec<String>)>> {
    let mut moved = Vec::new();
    for app in get("/v1/apps")?.as_array().into_iter().flatten() {
        let ours = app["bundle"].is_null() || app["bundle"] == app["built_in"];
        let had = ours && app["built_in"].is_string() && app["deploys"].as_u64().is_some_and(|n| n > 0);
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

/// Deploys the built-in apps as they are in `src`, where the network has them and what they're built from changed (an
/// app's folder, or one beside it that its links go to, which is why not only the folders named are looked at: an app
/// whose bundle is what's deployed already is left).
fn deploy_apps(src: &Path, changed: &[String]) -> Result<()> {
    if changed.is_empty() {
        return Ok(());
    }
    for app in get("/v1/apps")?.as_array().into_iter().flatten() {
        let Some(name) = app["name"].as_str().filter(|_| app["built_in"].is_string()) else { continue };
        let dir = src.join("apps").join(name);
        if !dir.join("app.js").is_file() {
            continue;
        }
        let files = one::app::bundle::sources(&dir)?;
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

/// Deploys to Cloudflare, with the build being sent out (`commit`), from the one of `linux` that is the Workers' home
/// (`Settings::at_home`; another refuses, which says so and is passed over): only there are its secrets
/// (`cloudflare deploy` in its home's cloudflare/). It waits for each to run that build, so it never
/// deploys the one before, and tries again what may pass (a link dropping as nodes restart, Cloudflare). Every network
/// has its Workers on Cloudflare, so it fails if no device deployed them. True if it failed.
fn deploy_cloudflare(linux: &[Value], commit: &str) -> bool {
    let home = format!("${{{}_HOME:-${{XDG_DATA_HOME:-$HOME/.local/share}}/{CLI}}}", CLI.to_uppercase());
    // The node's own build deploys it (what's called the CLI there may be something else), once it's the one sent
    // out: `exit::NOT_YET` says it isn't yet, `exit::NO_SETTINGS` that this device has none to deploy. It's the node that wrote its home's pid: Shell's program, which runs this on another
    // device, has its own folder in services/ as its home, so its home is two up, and its address (`_ADDR`) is the services' registry's, not the node's. As a login there would: its profile
    // is where 1Password's sign-in, which the Cloudflare secrets are read with, usually is, and ~/.local/bin where op may be.
    let deploy = format!(
        r#"h="{home}"; [ -f "$h/pid" ] || h=$(cd "$h/../.." && pwd); [ -f "$h/cloudflare/settings.json" ] || exit {no_settings}; node=/proc/$(cat "$h/pid")/exe
case "$("$node" version --json)" in *'"commit":"{commit}"'*) ;; *) echo "still on $("$node" version --json)"; exit {not_yet} ;; esac
. "$HOME/.profile" >/dev/null 2>&1; unset {var}_ADDR; {var}_HOME="$h" PATH="$HOME/.local/bin:$PATH" "$node" cloudflare deploy"#,
        var = CLI.to_uppercase(),
        no_settings = one::protocol::exit::NO_SETTINGS,
        not_yet = one::protocol::exit::NOT_YET,
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
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut failures = 0;
        let said = loop {
            let said = if local { run_here(&deploy) } else { shell(device, &deploy, 600) };
            let again = match &said {
                Err(_) => true,
                Ok(r) if r["exit"] == one::protocol::exit::NOT_YET => true,
                // Refused: another device is where the Workers are deployed from. None: this one has no settings.
                Ok(r) if r["exit"] == one::protocol::exit::REFUSED || r["exit"] == one::protocol::exit::NO_SETTINGS => false,
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
            Ok(r) if r["exit"] == one::protocol::exit::NO_SETTINGS => {}
            Ok(r) if r["exit"] == one::protocol::exit::REFUSED => println!("  {name}: {}", r["stderr"].as_str().unwrap_or("").trim()),
            Ok(r) if r["exit"] == 0 => {
                deployed = true;
                println!("  {name}: deployed to Cloudflare at {commit}");
            }
            Ok(r) => {
                failed = true;
                println!("  {name}: couldn't deploy to Cloudflare:\n{}{}", r["stdout"].as_str().unwrap_or(""), r["stderr"].as_str().unwrap_or("").trim());
            }
            Err(err) => {
                failed = true;
                let hint = if one::protocol::Coded::is(&err, one::protocol::code::NOT_INSTALLED) { " (add Shell in the App Store, or run one update on the device with the Cloudflare settings)" } else { "" };
                println!("  {name}: couldn't deploy to Cloudflare: {err:#}{hint}");
            }
        }
    }
    if !deployed && !failed {
        println!("  no device deployed to Cloudflare: none of them online has the settings");
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
    ensure!(timed("waited for after-update", || after.is_none_or(|mut after| after.wait())), "after-update failed");
    Ok(())
}

/// Deploys this device's release's version of each built-in app the network has, where it runs another.
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

/// Whether install.sh, which gets releases from GitHub, installs this custom build. Only that custom build is released there.
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
        step(Command::new("git").args(["clone", "-q", CONFIG_REPO]).arg(&dir))?;
    }
    for args in [&["fetch", "-q", "origin", "HEAD"][..], &["checkout", "-q", "--detach", "--force", "FETCH_HEAD"]] {
        step(Command::new("git").current_dir(&dir).args(args))?;
    }
    Ok((dir.join(CONFIG_FILE), Some(git(&dir, &["rev-parse", "HEAD"])?)))
}

/// Builds the Cloudflare node (`cloudflare/node/`) into WebAssembly and its JavaScript glue, for the release to carry: → the
/// folder they're in. It needs Rust's wasm32-unknown-unknown target, a clang that builds for it (SQLite is C), and the
/// wasm-bindgen of the version it's built with. `profile` is cargo's: a release's is `release`, a throwaway network's `dev`.
pub fn cloudflare_node(src: &Path, config: &Path, profile: &str) -> Result<PathBuf> {
    run_quiet(Command::new(cargo()).current_dir(src).env("CONFIG", config).args(["build", "--profile", profile, "-p", "one-cloudflare-node", "--target", "wasm32-unknown-unknown"]))
        .context("build the Cloudflare node (rustup target add wasm32-unknown-unknown)")?;
    let out = src.join("target/cloudflare-node");
    let wasm = src.join("target/wasm32-unknown-unknown").join(if profile == "dev" { "debug" } else { profile }).join("one_cloudflare_node.wasm");
    // Its glue is only made again for a node cargo built again, as the client's is: a build that holds it (`CLOUDFLARE_NODE`)
    // watches these files, so rewriting them would have that build done again.
    let modified = |path: &Path| path.metadata().and_then(|m| m.modified()).ok();
    let glued = out.join("one_cloudflare_node_bg.wasm");
    if modified(&glued).is_some_and(|glued| modified(&wasm).is_some_and(|built| glued > built)) {
        return Ok(out);
    }
    run_quiet(Command::new("wasm-bindgen").args(["--target", "web", "--out-dir"]).arg(&out).arg(&wasm)).context("make its glue (cargo install wasm-bindgen-cli, the version in Cargo.lock)")?;
    Ok(out)
}

/// Builds the client (`client/`) into WebAssembly for browsers, as small as it builds, and its JavaScript glue: → the
/// folder they're in. It needs what `cloudflare_node` does, and makes it smaller still with wasm-opt where it's installed.
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

/// Go, also where mise's shims put it, for a shell that doesn't have it on its PATH.
fn go() -> PathBuf {
    let mise = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/mise/shims/go");
    if quiet(Command::new("go").arg("version")) || !mise.exists() { "go".into() } else { mise }
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
    let me = state["network"]["user"].clone();
    state["network"]["devices"].as_array().into_iter().flatten().filter(|d| d["user"] == me).cloned().collect()
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

/// What an update says last, when the Macs are still building in the background.
fn says_wait(short: &str) {
    println!("The Macs build it in the background: {CLI} status --until {short} waits for them, and for every device's services.");
}

/// `status --until <commit> [--device <name>]… [--timeout <time>]`, with the words after `status`.
pub fn wait(args: &[String]) -> Result<()> {
    let (mut commit, mut names, mut limit) = (None, Vec::new(), Duration::from_secs(10 * 60));
    let mut words = args.iter().skip_while(|a| *a != "status").skip(1);
    while let Some(flag) = words.next() {
        let typed = |m: String| anyhow::Error::from(crate::tools::Failed::Typed(m));
        let value = words.next().filter(|v| !v.starts_with("--")).ok_or_else(|| typed(format!("{flag} takes a value: see {CLI} status --help")))?;
        match flag.as_str() {
            "--until" => commit = Some(value.to_lowercase()),
            "--device" => names.extend(value.split(',').map(str::to_string)),
            "--timeout" => {
                let (n, unit) = value.split_at(value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len()));
                let n: u64 = n.parse().map_err(|_| typed("--timeout is like 90s, 10m or 1h".into()))?;
                limit = Duration::from_secs(n * match unit { "" | "s" => 1, "m" => 60, "h" => 3600, _ => return Err(typed("--timeout is like 90s, 10m or 1h".into())) });
            }
            other => return Err(typed(format!("{other} isn't an input of status --until: see {CLI} status --help"))),
        }
    }
    let commit = commit.unwrap_or_default();
    ensure!(commit.len() >= 7 && commit.chars().all(|c| c.is_ascii_hexdigit()), "give the commit: {CLI} status --until $(git rev-parse HEAD)");
    wait_until(&commit, &names, limit)
}

/// Waits until each of the user's devices (or those named) runs `commit`, or a build with it in, and its services
/// answer, or a Mac's build of it fails; says what each did, and fails unless all made it. Shell on a Mac says how its
/// builds went, which `follow` set up; a device that is off or restarting is waited for like the rest.
fn wait_until(commit: &str, names: &[String], limit: Duration) -> Result<()> {
    let short = &commit[..commit.len().min(12)];
    // Says so when no node answers, which later polls (a node restarting on the new build) never do.
    get("/v1/state")?;
    let all: Vec<Value> = devices();
    ensure!(!all.is_empty(), "no devices are listed here yet");
    for n in names {
        ensure!(all.iter().any(|d| d["name"] == n.as_str()), "no device named {n}: {}", all.iter().filter_map(|d| d["name"].as_str()).collect::<Vec<_>>().join(", "));
    }
    // A device by its key and name, as it was at the start: and the build it was on, for what it changed from.
    let mut open: Vec<(String, String, String)> =
        all.iter().filter(|d| names.is_empty() || names.iter().any(|n| d["name"] == n.as_str())).map(|d| (d["key"].as_str().unwrap_or("").into(), d["name"].as_str().unwrap_or("").into(), d["build"].as_str().unwrap_or("?").into())).collect();
    let (started, deadline) = (Instant::now(), Instant::now() + limit);
    let (mut fetched, mut reasons, mut asked, mut phones) = (std::collections::HashSet::new(), std::collections::HashMap::new(), std::collections::HashMap::<String, Instant>::new(), std::collections::HashMap::<String, String>::new());
    let mut failed = false;
    loop {
        let now = devices();
        open.retain(|(key, name, was)| {
            let Some(d) = now.iter().find(|d| d["key"] == key.as_str()) else { return true };
            let build = d["build"].as_str().unwrap_or("");
            let mac = d["target"].as_str().is_some_and(|t| t.ends_with("-apple-darwin"));
            // What the Mac's build job says, not more than every few seconds: how its build is going, and its iPhone.
            let mut said = None;
            let running = has(commit, build, &mut fetched);
            if mac && d["online"] == true && asked.get(key).is_none_or(|t| t.elapsed() > Duration::from_secs(6)) {
                asked.insert(key.clone(), Instant::now());
                said = mac_said(d);
                if let Some(phone) = said.as_ref().and_then(|s| s.phone.clone()) {
                    phones.insert(key.clone(), phone);
                }
            }
            if running {
                // Switched on and not yet answering: starting, or what went wrong.
                let troubled: Vec<String> = d["parts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|p| p["on"] == true)
                    .filter_map(|p| {
                        let why = p["problem"].as_str().map(str::to_string).or_else(|| p["doing"].as_str().map(|d| d.to_lowercase()))?;
                        Some(format!("{}: {}", p["name"].as_str()?, why.trim_end_matches('.')))
                    })
                    .collect();
                if troubled.is_empty() {
                    if was == build {
                        println!("  {name}: already running {build}");
                    } else {
                        println!("  {name}: {was} → {build} ({}s)", started.elapsed().as_secs());
                    }
                    if let Some(phone) = phones.get(key) {
                        println!("    {phone}");
                    }
                    return false;
                }
                reasons.insert(key.clone(), format!("runs {build}, but its services aren't all answering: {}", troubled.join("; ")));
                return true;
            }
            if let Some(s) = said.as_ref() {
                if let Some(log) = &s.failed_log {
                    failed = true;
                    println!("  {name}: its build of {} failed: the end of its log, ~/Library/Caches/{CLI}/update.log on {name}, is", &s.tried[..s.tried.len().min(12)]);
                    for line in log.lines() {
                        println!("      {line}");
                    }
                    if let Some(phone) = phones.get(key) {
                        println!("    {phone}");
                    }
                    return false;
                }
                reasons.insert(key.clone(), s.waiting_for(commit, short, build, &mut fetched));
            } else if d["online"] != true {
                reasons.insert(key.clone(), format!("offline, on {build}; it updates itself when it's back"));
            } else if !mac {
                reasons.insert(key.clone(), format!("still on {build}"));
            }
            true
        });
        if open.is_empty() {
            ensure!(!failed, "not every device runs {short}");
            return Ok(());
        }
        if Instant::now() > deadline {
            for (key, name, _) in &open {
                println!("  {name}: {}", reasons.get(key).map_or("not answering".to_string(), |r| r.clone()));
            }
            println!("Gave up waiting for {short} after {}s.", limit.as_secs());
            bail!("not every device runs {short}");
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// Whether `build` (what a device runs) is `commit` or has it in: a later update sends main's tip, which holds it.
/// Asks this folder's git, which fetches once for a build it hasn't got.
fn has(commit: &str, build: &str, fetched: &mut std::collections::HashSet<String>) -> bool {
    if build.is_empty() {
        return false;
    }
    if commit.starts_with(build) || build.starts_with(commit) {
        return true;
    }
    let ancestor = || Command::new("git").args(["merge-base", "--is-ancestor", commit, build]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().ok().and_then(|s| s.code());
    match ancestor() {
        Some(0) => true,
        Some(1) => false,
        _ if fetched.insert(build.to_string()) => {
            quiet(Command::new("git").args(["fetch", "-q", "origin"]));
            ancestor() == Some(0)
        }
        _ => false,
    }
}

/// What a Mac's update job files say: the build it was last asked for (`want`) and tried, what it built, and its iPhone.
struct MacSaid {
    want: String,
    tried: String,
    built: String,
    failed_log: Option<String>,
    phone: Option<String>,
}

fn mac_said(mac: &Value) -> Option<MacSaid> {
    let script = format!(
        r#"c="$HOME/Library/Caches/{CLI}"
for f in want built tried phone.built phone.installed phone.failed; do printf '%s=%s\n' "$f" "$(cat "$c/$f" 2>/dev/null)"; done
echo ---; tail -n 8 "$c/failed.log" 2>/dev/null"#
    );
    let r = shell(mac, &script, 30).ok().filter(|r| r["exit"] == 0)?;
    let out = r["stdout"].as_str()?;
    let (values, log) = out.split_once("---\n")?;
    let get = |key: &str| values.lines().find_map(|l| l.strip_prefix(&format!("{key}="))).unwrap_or("").trim().to_string();
    let commit_of = |v: String| v.split(' ').next().unwrap_or("").to_string();
    let (want, tried, built) = (commit_of(get("want")), commit_of(get("tried")), commit_of(get("built")));
    let (phone_built, installed, phone_failed) = (commit_of(get("phone.built")), commit_of(get("phone.installed")), commit_of(get("phone.failed")));
    let twelve = |c: &str| c[..c.len().min(12)].to_string();
    let phone = match (phone_built.is_empty(), installed.is_empty()) {
        _ if !phone_failed.is_empty() => Some(format!("iPhone: its build of {} failed", twelve(&phone_failed))),
        (true, true) => None,
        _ if phone_built != installed && !phone_built.is_empty() => Some(format!("iPhone: has {}, {} built and installs when it's reachable and unlocked", if installed.is_empty() { "nothing".to_string() } else { twelve(&installed) }, twelve(&phone_built))),
        _ => Some(format!("iPhone: has {}", twelve(&installed))),
    };
    // Failed: the last thing it was asked is what it tried, and that isn't what it built (a newer ask would differ).
    let failed = !tried.is_empty() && tried == want && tried != built;
    Some(MacSaid { failed_log: failed.then(|| log.trim_end().to_string()), want, tried, built, phone })
}

impl MacSaid {
    /// Why a Mac that doesn't run `commit` yet doesn't.
    fn waiting_for(&self, commit: &str, short: &str, build: &str, fetched: &mut std::collections::HashSet<String>) -> String {
        let twelve = |c: &str| c[..c.len().min(12)].to_string();
        if has(commit, &self.built, fetched) {
            format!("built {}, its node is restarting on it", twelve(&self.built))
        } else if has(commit, &self.want, fetched) {
            format!("building {} (on {build})", twelve(&self.want))
        } else {
            format!("on {build}; nothing has asked it to build {short} yet: {CLI} update does")
        }
    }
}

/// A command in this machine's own sh, answered as `shell` answers: its exit and what it said.
fn run_here(command: &str) -> Result<Value> {
    let out = Command::new("sh").args(["-c", command]).output().context("run sh")?;
    Ok(json!({ "exit": out.status.code().unwrap_or(-1), "stdout": String::from_utf8_lossy(&out.stdout), "stderr": String::from_utf8_lossy(&out.stderr) }))
}

fn shell(device: &Value, command: &str, timeout: u64) -> Result<Value> {
    // Shell's service on that device, by its name: its own program there, or built into an older node.
    let r = post("/v1/call", json!({ "device": device["name"], "service": "shell", "app": "shell", "op": "run", "input": { "command": command, "timeout": timeout } }))?;
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
CONFIG="$config" client/native/disk.sh retry client/native/mac/build.sh --release
{assets}
JOB
{start}"#,
        checkout = mac_checkout(commit, config_commit.unwrap_or("")),
        start = mac_start("update.sh")
    )
}

/// Shell for a Mac: has it build `commit`'s app now, and install it, then the iPhone app, if what that's made of changed,
/// for the iPhone, all in one job in its logged-in session: a newer update stops the build it's on and builds that
/// instead. It installs the iPhone app whenever the phone is reachable, and keeps trying until it is. A job already
/// running takes this update's build scripts at its next build, and tries a build that failed again if they changed.
/// Says how its last builds failed, once, and what the iPhone has.
fn mac_follow(commit: &str, config_commit: Option<&str>) -> String {
    let short = &commit[..commit.len().min(12)];
    format!(
        r#"set -e
c="$HOME/Library/Caches/{CLI}"; label=app.{CLI}.update
mkdir -p "$c"; cd "$c"
if [ -f failed ]; then echo "its build of $(cut -c1-12 failed) failed:"; sed 's/^/    /' failed.log; rm -f failed; fi
if [ -f phone.failed ]; then echo "its iPhone build of $(cut -c1-12 phone.failed) failed:"; sed 's/^/    /' phone.failed.log; rm -f phone.failed; fi
if [ -s phone.built ]; then
	b=$(cut -c1-12 phone.built)
	if [ "$(cat phone.built)" = "$(cat phone.installed 2>/dev/null)" ]; then echo "iPhone: installed $b"; else echo "iPhone: not reachable, will install $b when it is"; fi
fi
echo "builds {short} now, instead of anything older, then the iPhone app if it changed"
# The scripts are replaced whole, so a job that's running them keeps the old ones until its next build takes the new.
put() {{ cat > "$1.new"; mv "$1.new" "$1"; }}
put build.sh <<'JOB'
set -e
c="$HOME/Library/Caches/{CLI}"
{checkout}
CONFIG="$config" client/native/disk.sh retry client/native/mac/build.sh
JOB
put phone.sh <<'JOB'
set -e
c="$HOME/Library/Caches/{CLI}"
{checkout}
# Only what the iPhone app is made of: the Mac app's but for what's the Mac's own (its window's code, its node's
# services and helpers, apps), and each network closes the app on the phone.
last=$(cat "$c/phone.built" 2>/dev/null || true)
if [ "${{last#* }}" = "${{2:-}}" ] && git diff --quiet "${{last%% *}}" "$1" -- . ':!client/native/mac' ':!client/native/windows' ':!client/native/android' ':!service' ':!apps' ':!docs' ':!tests' ':!*.md' 2>/dev/null; then exit 3; fi
CONFIG="$config" client/native/disk.sh retry client/native/iphone/build.sh device "$c/phone"
JOB
put follow.sh <<'JOB'
# launchd runs this in the login shell, zsh on a Mac; the job control it uses is bash's.
[ -n "$BASH_VERSION" ] || exec /bin/bash "$0" "$@"
c="$HOME/Library/Caches/{CLI}"
# Each build in a process group of its own, which a newer update stops (client/native/mac/build.sh's install ignores that).
set -m
want() {{ cat "$c/want"; }}
# A build that failed is tried again when its scripts change (a newer update's, if the job is running), and not otherwise.
scripts() {{ cat "$c/build.sh" "$c/phone.sh" | cksum; }}
retry() {{ [ -f "$c/tried.scripts" ] && [ "$(scripts)" != "$(cat "$c/tried.scripts")" ]; }}
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
	if [ "$want" != "$(cat "$c/tried" 2>/dev/null)" ] || retry; then
		with=$(scripts); rm -f "$c/tried" "$c/tried.scripts"
		# zsh doesn't split "$want" into its commit and config: this is bash.
		set -- ${{want%% *}} "${{want#* }}"
		run "$c/build.sh" "$1" "$2" > "$c/update.log" 2>&1; status=$?
		[ -z "$stopped" ] || continue
		if [ $status = 0 ]; then echo "$want" > "$c/built"; else
			tail -n 25 "$c/update.log" > "$c/failed.log"; echo "$want" | tee "$c/failed" > "$c/tried"; echo "$with" > "$c/tried.scripts"; continue
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
	for _ in $(seq 30); do sleep 2; if [ "$(want)" != "$want" ] || retry; then break; fi; done
done
JOB
# The scripts first: a running job stops its build for this and starts the next with them.
echo "{commit} {config}" > want
pid=$(launchctl list | awk -v l="$label" '$3 == l {{ print $1 }}')
if [ -n "$pid" ] && [ "$pid" != "-" ]; then exit 0; fi
{start}"#,
        config = config_commit.unwrap_or(""),
        checkout = mac_checkout("$1", "${2:-}"),
        start = mac_start("follow.sh")
    )
}

/// Shell that puts the Mac's clean clone at `commit` and sets `config` to the config to build with: the clone's own, or
/// for a custom build whose config is in its own repo, that repo at `config_commit`.
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
git clean -qfdx -e /target -e /client/native/target
# What the builds leave in target/ is kept in bounds, and a Mac with no room says so here, not in a link step.
client/native/disk.sh prune
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
