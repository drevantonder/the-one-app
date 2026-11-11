//! `throwaway`: a throwaway network, a network of its own, made as `create` makes every new one, by a node beside this machine's own with its own
//! home and ports, so it touches nothing of the user's (`one::mode`). There's one for each checkout it's run in, from any folder in it, so threads working in different ones each have their
//! own. Its Workers run beside it, and its auth lets its first user in with no passkey (cloudflare::Settings::throwaway_user), so a
//! client of this checkout's build, on a device borrowed for it (`throwaway client`), signs in with nobody there; others join
//! as anyone does, by an invite link the first makes, and again with no passkey (`throwaway client <kind> --as <name>`). It
//! keeps its data until `throwaway stop`: left alone it stops, and `throwaway` starts it again as it was, as it does when this build
//! is newer than the one it runs, unless a client session (`throwaway client`) is running, which a new build must not break:
//! then at the next command with none. Starting it again leaves its clients' browsers alone.


use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use one::config::CLI;
use one::mode::Throwaway;
use one::release::Version;

use crate::dev;
use serde_json::json;

mod clients;
mod devices;
mod network;
mod rebuild;
pub(crate) mod sign_in;

pub(crate) use network::node_at;
use network::{older_build, no_client_running, running, start, stop, stop_some};

fn is_checkout(top: &Path) -> bool {
    top.join(".git").exists() && top.join("Cargo.toml").exists() && top.join("ARCHITECTURE.md").exists()
}

/// The checkout this program was built in (`<checkout>/target/<profile>/`), if it was.
fn built_in() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    Some(exe.parent()?.parent()?.parent()?.to_path_buf()).filter(|top| is_checkout(top))
}

/// The throwaway network for the checkout this build is from (`<checkout>/target/<profile>/`), else for the checkout the
/// current folder is in, wherever in it, as git finds it, beside the downloads every throwaway network shares: → its folder,
/// and the checkout. So a checkout's own build finds its throwaway network from any folder, as an MCP client starts it, and
/// from a folder in no checkout there is none: nothing starts a network for a folder that isn't one.
fn folder() -> Option<(PathBuf, PathBuf)> {
    let base = PathBuf::from(format!("{}-throwaway", one::home().display()));
    let built = built_in();
    let top = || std::process::Command::new("git").args(["rev-parse", "--show-toplevel"]).output().ok().filter(|o| o.status.success()).map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())).filter(|top| is_checkout(top));
    let checkout = built.or_else(top)?;
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(checkout.to_string_lossy().as_bytes()).into();
    let hash = one::keys::hex(&hash);
    let name = checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Some((base.join(format!("{name}-{}", &hash[..6])), checkout))
}

/// A command typed after `throwaway`: it makes sure the throwaway network runs and, but for the commands that manage it, runs
/// the command as one of its programs, by the `one` program built with this one (`Throwaway::tell`).
pub fn run(rest: &[String]) -> Result<()> {
    let (build, ui, rest) = chosen(rest)?;
    if matches!(rest.first().map(String::as_str), Some("--help" | "-h")) || (rest.len() == 1 && rest[0] == "help") || (rest.len() > 1 && matches!(rest[0].as_str(), "stop" | "client" | "publish" | "update") && rest.iter().any(|a| a == "--help" || a == "-h")) {
        println!("{}", usage());
        return Ok(());
    }
    let Some((dir, checkout)) = folder() else {
        bail!("{} throwaway is for a checkout of The One App, and this folder isn't in one: run it in the checkout, or any folder in it", dev())
    };
    // A browser (the web client, and anyone `--as` joins in one) and the Cloudflare node need the WebAssembly, which a
    // throwaway network running this checkout's build reads from the checkout, as it reads its pages from its UI folder.
    let given = ui.is_some();
    let mut ui = ui;
    let words: Vec<&str> = rest.iter().map(String::as_str).collect();
    let browser = words.first() == Some(&"client") && (words.get(1) == Some(&"web") || words.contains(&"--as"));
    if build.is_none() && (browser || words == ["cloudflare", "node", "deploy"]) {
        rebuild::web_client_and_cloudflare_node(&checkout)?;
        ui = ui.or_else(|| Throwaway::ui_in(&dir).is_none().then(|| checkout.clone()));
    }
    let starting = network::exclusive(&dir)?;
    network::choose(&dir, build.as_deref(), ui.as_deref())?;
    match rest.first().map(String::as_str) {
        Some("stop") => {
            devices::give_back(&dir, &checkout);
            stop(&dir, true)?;
            println!("Stopped and deleted this checkout's throwaway network, and gave back its devices. {} throwaway starts a new one.", dev());
            return Ok(());
        }
        Some("cloudflare") if rest.get(1).map(String::as_str) == Some("serve") => bail!("the throwaway network runs its own Workers"),
        _ => {}
    }
    let addr = match running(&dir) {
        Some(addr) if !older_build(&dir) => addr,
        Some(addr) if !no_client_running(&dir) => {
            eprintln!("This build is newer than the one the throwaway network runs, and a client of it is running: it starts again at the next command once that's done.");
            addr
        }
        Some(_) => {
            eprintln!("This build is newer than the one the throwaway network runs: starting it again (it keeps its data, and its clients' browsers stay)…");
            start(&dir)?
        }
        None => start(&dir)?,
    };
    network::read_pages_again(&dir)?;
    if given {
        eprintln!("Its pages and screens are read from there now, and again by each command here. Its Workers' own code is read when they start: {} throwaway restart, once you've changed that.", dev());
    }
    if rest.iter().map(String::as_str).eq(["cloudflare", "node", "deploy"]) {
        return cloudflare_node(&dir);
    }
    if rest.first().map(String::as_str) == Some("restart") {
        start(&dir)?;
        return Ok(());
    }
    match rest.first().map(String::as_str) {
        Some("publish") => return publish(&dir, &addr),
        Some("update") => return update(&dir, &addr),
        _ => {}
    }
    drop(starting);
    match rest.first().map(String::as_str) {
        None => return say_where(&dir, &addr),
        Some("client") => {
            let settings = one::cloudflare::Settings::load_in(&dir)?;
            return clients::client(&dir, &checkout, &settings, &rest[1..]);
        }
        Some("git") => return git(&dir, &addr, &rest[1..]),
        _ => {}
    }
    // As one of its programs: its home, so the CLI asks its node with its token, as nobody signed in, and its Cloudflare
    // settings are its own Workers'; and its mode, so nothing of this machine's own network's is touched.
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(network::program(&dir)?);
    Throwaway::tell(&mut command, &dir, &addr);
    Err(anyhow::Error::from(command.args(rest).exec()).context("running the command in the throwaway network"))
}

/// `--build <program>` and `--ui <folder>`, given before the command, as full paths: → them, and what follows them.
fn chosen(rest: &[String]) -> Result<(Option<PathBuf>, Option<PathBuf>, &[String])> {
    let (mut build, mut ui, mut rest) = (None, None, rest);
    while let [flag, value, more @ ..] = rest
        && (flag == "--build" || flag == "--ui")
    {
        let path = std::fs::canonicalize(value).with_context(|| format!("no {value}"))?;
        if flag == "--build" { build = Some(path) } else { ui = Some(path) }
        rest = more;
    }
    Ok((build, ui, rest))
}

/// Publishes this checkout's build at the throwaway network's address, under its own release key, for its device to take
/// with `update`: it must be a different commit from the one the device runs, which `--build` starts it on.
fn publish(dir: &Path, addr: &str) -> Result<()> {
    let version = Version::of(&crate::program()?).context("ask this checkout's build what it is")?;
    let runs = Version::of(&network::program(dir)?).context("ask the throwaway network's program what it is")?;
    anyhow::ensure!(version.commit != runs.commit, "the throwaway network runs this build already ({}), so it would have nothing to take: start it on an older build with {} throwaway --build <program>, or commit and build again", runs.commit, dev());
    publish_build(dir, addr)?;
    println!("Published {} under this throwaway network's own release key. {} throwaway update takes it, and Cloudflare's Update in Devices its Workers.", version.commit, dev());
    Ok(())
}

/// This checkout's build, published at the throwaway network's address under its own release key: what its devices
/// update to, and what a computer the desktop app sets up takes.
pub(crate) fn publish_build(dir: &Path, addr: &str) -> Result<()> {
    let build = crate::program()?;
    let version = Version::of(&build).context("ask this checkout's build what it is")?;
    let (node, key) = (node_at(dir, addr), Throwaway::own_release_key(dir)?);
    one::build::publish(&node, &build, &key, &version, None)?;
    // The Workers and the web client too, which Cloudflare's Update in Devices takes from the record.
    let bundle = |files| one::web::bundle::Bundle { commit: version.commit.clone(), committed: version.committed, shell: one::web::SHELL, files };
    let (workers, web) = (dir.join("workers.bundle"), dir.join("web.bundle"));
    std::fs::write(&workers, bundle(one::ui_folder::cloudflare()).packed()?)?;
    std::fs::write(&web, one::web::bundle::signed(&key, &bundle(one::web::with_client()).packed()?)?)?;
    one::build::publish_latest(&node, &key, &[(one::latest::Thing::WORKERS.into(), Some(workers), version.clone(), "-".into()), (one::latest::Thing::WEB.into(), Some(web), version.clone(), "-".into())])
}

/// `update`, in the throwaway network: nothing supervises its oned, so what systemd or launchd does for a device's, this does
/// here, starting it again unless it runs the program's build already (its oned may have taken it by itself first): once
/// the oned asked to stop has stopped its onet and app runtime and gone, on the same address, its Workers left running.
fn update(dir: &Path, addr: &str) -> Result<()> {
    let program = network::program(dir)?;
    let mut command = std::process::Command::new(&program);
    Throwaway::tell(&mut command, dir, addr);
    let status = command.arg("update").status().context("couldn't run the update")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    let now = Version::of(&program).context("ask the throwaway network's program what it is")?;
    let runs = std::fs::read_to_string(dir.join(one::update::RUNNING)).unwrap_or_default();
    if runs.trim() == now.commit && running(dir).is_some() {
        return Ok(());
    }
    let oned = std::fs::read_to_string(dir.join("pid")).ok().and_then(|p| p.trim().parse::<i32>().ok());
    let started = std::time::Instant::now();
    // It stops onet and its app runtime first, ten seconds each at most; nothing says when it's gone.
    while oned.is_some_and(|pid| unsafe { libc::kill(pid, 0) } == 0) {
        anyhow::ensure!(started.elapsed() < std::time::Duration::from_secs(30), "the throwaway network's oned didn't stop: node.log says why");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    network::spawn_node(dir, Some(addr)).map(drop)
}

fn say_where(dir: &Path, addr: &str) -> Result<()> {
    let state = node_at(dir, addr).state()?;
    let (me, address) = (state.looking(), state.address().unwrap_or(""));
    println!(
        "This checkout's throwaway network is running on this machine: a new network, Throwaway, with only what every new network
starts with, and {me} as its admin, who signs in at {address} with no passkey. Its folder is {}, with its
recovery key; its Workers' settings are in {}.

{CLI}-dev throwaway --help lists its commands.",
        dir.display(),
        dir.join("cloudflare").join("settings.json").display()
    );
    Ok(())
}

/// `git`, told that the throwaway network's repos (`<its address>/repos/…`) are reached through this build's remote helper, as
/// git is on a computer with a node: this build's helper is linked in a folder of the throwaway network's, put
/// first on git's PATH (the one the real network's CLI links is another build, reaching another node).
fn git(dir: &Path, addr: &str, args: &[String]) -> Result<()> {
    let address = node_at(dir, addr).state()?.address().context("the throwaway network has no address")?.trim_end_matches('/').to_string();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin)?;
    let helper = bin.join(one::git_wire::helper());
    // Replaced in one step: another `git` here at once has this one on its PATH while it's linked.
    let linking = bin.join(format!("{}.{}", one::git_wire::helper(), std::process::id()));
    std::os::unix::fs::symlink(network::program(dir)?, &linking)?;
    std::fs::rename(&linking, &helper)?;
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())))?;
    let config: usize = std::env::var("GIT_CONFIG_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let (key, value) = one::git_wire::instead_of(&address);
    let mut command = std::process::Command::new("git");
    Throwaway::tell(&mut command, dir, addr);
    let status = command
        .args(args)
        .env("PATH", path)
        .env("GIT_CONFIG_COUNT", (config + 1).to_string())
        .env(format!("GIT_CONFIG_KEY_{config}"), key)
        .env(format!("GIT_CONFIG_VALUE_{config}"), value)
        .status()
        .context("couldn't run git")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Runs the throwaway network's Cloudflare node beside its Workers, in their workerd, as `cloudflare node deploy` puts one in
/// Cloudflare: its devices answer its endpoint.
fn cloudflare_node(dir: &Path) -> Result<()> {
    let mut settings = one::cloudflare::Settings::load_in(dir)?;
    if !settings.cloudflare_node {
        settings.cloudflare_node = true;
        settings.save()?;
        stop_some(dir, false, false)?;
        start(dir)?;
    }
    let addr = running(dir).context("the throwaway network didn't start again")?;
    let key = settings.cloudflare_node_endpoint().context("the throwaway network's Workers have no Cloudflare node key")?;
    node_at(dir, &addr).post("/api/settings", json!({ "name": one::database::scopes::CLOUDFLARE_NODE, "value": key }))?;
    println!("The throwaway network's Cloudflare node runs beside its Workers, its endpoint {}.", &key[..8]);
    Ok(())
}

/// `throwaway`'s own help, with the MCP command as one that finds this throwaway network from any folder: this build,
/// if it's a checkout's own.
fn usage() -> String {
    let mcp = match folder() {
        Some((_, checkout)) => match std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).filter(|e| e.starts_with(&checkout)) {
            Some(exe) => format!("{} throwaway mcp", exe.display()),
            None => format!("sh -c 'cd {} && exec {CLI}-dev throwaway mcp'", checkout.display()),
        },
        None => mcp_anywhere(),
    };
    format!("Usage:\n{}", one::cli::help_as(&dev(), &commands(&mcp).help))
}

fn mcp_anywhere() -> String {
    format!("sh -c 'cd <checkout> && exec {CLI}-dev throwaway mcp'")
}

/// `throwaway`'s help, in `help all`: it runs before the words after it are read, as their own command (`run`).
pub fn command() -> one::cli::Command {
    commands(&mcp_anywhere())
}

fn commands(mcp: &str) -> one::cli::Command {
    let form = |usage: &str, about: String| (usage.to_string(), about);
    one::cli::Command {
        help: vec![
            form("throwaway [--build <program>] [--ui <folder>]", "start this checkout's throwaway network, if it isn't running (or runs an older build), and say where it is: on this machine beside yours, with only what every new one starts with, to look around and test in. --build runs that program of a network (by default the one beside this), and --ui reads its pages and Workers from a checkout, as they are there, instead of what the build holds: its pages and screens are read again by each command here, whether or not it's running already".into()),
            form("throwaway restart", "start it again, keeping its data, to pick up a changed Worker in the UI folder".into()),
            form("throwaway <command>", format!("any command, in it: {CLI}-dev throwaway status, {CLI}-dev throwaway apps, {CLI}-dev throwaway repos list")),
            form("throwaway mcp", format!("its MCP server, for an agent: claude mcp add {CLI}-throwaway -- {mcp}")),
            form("throwaway git <args>", format!("git, reaching its repos at its address: {CLI}-dev throwaway git clone <address>/repos/<owner>/<repo>.git, then {CLI}-dev throwaway git push in the clone")),
            form("throwaway cloudflare node deploy", "run its Cloudflare node beside its Workers, as Cloudflare would".into()),
            form("throwaway client <kind>", "a signed-in client of this checkout's build: web, linux, mac, iphone, android or windows. mac and iphone build on the Mac and stop that build if the command is killed or takes 9 minutes: run it again to build on from what was built".into()),
            form("throwaway client <kind> --as sam", "the same, as someone else, invited by you with Add Someone's link".into()),
            form("throwaway publish", "publish this checkout's build to it under its own release key, for its device to take, and Update on Cloudflare in Devices to redeploy its Workers with; give it an older build first with --build".into()),
            form("throwaway update", format!("take what's published, as {CLI} update does on a device, and start it again on it")),
            form("throwaway stop", format!("stop and delete it, and give back its devices; {CLI}-dev throwaway then starts a new one")),
        ],
        run: |_| Ok(false),
    }
}
