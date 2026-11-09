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

use crate::dev;
use serde_json::json;

mod clients;
mod devices;
mod network;
mod rebuild;
pub(crate) mod sign_in;

pub(crate) use network::node_at;
use network::{ older_build, no_client_running, running, start, stop, stop_some};

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
    if matches!(rest.first().map(String::as_str), Some("--help" | "-h")) || (rest.len() == 1 && rest[0] == "help") || (rest.len() > 1 && matches!(rest[0].as_str(), "stop" | "client" | "sign-in") && rest.iter().any(|a| a == "--help" || a == "-h")) {
        println!("{}", usage());
        return Ok(());
    }
    let Some((dir, checkout)) = folder() else {
        bail!("{} throwaway is for a checkout of The One App, and this folder isn't in one: run it in the checkout, or any folder in it", dev())
    };
    // These two need what only this checkout's debug build made with them holds; the first time in a checkout there is none.
    if build.is_none() && (rest.iter().map(String::as_str).eq(["client", "web"]) || rest.iter().map(String::as_str).eq(["cloudflare", "node", "deploy"])) {
        // This program is the build it replaces (a build of this checkout, named before it's replaced), and writes the pages a UI folder
        // is served as (`read_pages_again`): the new one runs this command, so those pages hold what its Workers' do.
        let this = built_in().and(std::env::current_exe().ok());
        if rebuild::with_web_client_and_cloudflare_node(&checkout)?
            && let Some(this) = this
        {
            use std::os::unix::process::CommandExt;
            return Err(std::process::Command::new(this).args(std::env::args_os().skip(1)).exec().into());
        }
    }
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
    if ui.is_some() {
        eprintln!("Its pages and screens are read from there now, and again by each command here. Its Workers' own code is read when they start: {} throwaway restart, once you've changed that.", dev());
    }
    if rest.iter().map(String::as_str).eq(["cloudflare", "node", "deploy"]) {
        return cloudflare_node(&dir);
    }
    match rest.first().map(String::as_str) {
        None => return say_where(&dir, &addr),
        Some("client") => {
            let settings = one::cloudflare::Settings::load_in(&dir)?;
            return clients::client(&dir, &checkout, &settings, &rest[1..]);
        }
        Some("restart") => {
            start(&dir)?;
            return Ok(());
        }
        Some("sign-in") => return sign_in(&dir, &addr),
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

/// `build --dev`: this checkout's debug build with the web client and the Cloudflare node in it.
pub fn build_dev() -> Result<()> {
    let (_, checkout) = folder().context("run this in a checkout of The One App")?;
    rebuild::with_web_client_and_cloudflare_node(&checkout).map(drop)
}

/// `sign-in`, as the throwaway network's user, with no passkey: its CLI signs in as any does, with its own git config and
/// no screen, and the link it gives is approved here, as its auth lets a throwaway network's first user in with none.
fn sign_in(dir: &Path, addr: &str) -> Result<()> {
    use std::io::BufRead;
    let address = node_at(dir, addr).state()?.address().context("the throwaway network has no address")?.to_string();
    let mut command = std::process::Command::new(network::program(dir)?);
    Throwaway::tell(&mut command, dir, addr);
    let mut child = command.arg("sign-in").env("GIT_CONFIG_GLOBAL", dir.join("gitconfig")).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY").stdout(std::process::Stdio::piped()).spawn()?;
    for line in std::io::BufReader::new(child.stdout.take().context("no output from signing in")?).lines() {
        let line = line?;
        let said = line.trim();
        if said.starts_with(&address) && said.contains("/sign-in") {
            sign_in::approve_on_throwaway(&address, said, None)?;
        }
    }
    anyhow::ensure!(child.wait()?.success(), "its CLI didn't sign in");
    let me = node_at(dir, addr).state()?.user.map_or("you".into(), |u| u.name);
    println!("Signed in as {me}. {} throwaway and {} throwaway mcp act as them until {} throwaway sign-out.", dev(), dev(), dev());
    Ok(())
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
/// git is on a computer signed in to any network: this build's helper is linked in a folder of the throwaway network's, put
/// first on git's PATH (the one the real network's CLI links is another build, reaching another node).
fn git(dir: &Path, addr: &str, args: &[String]) -> Result<()> {
    let address = node_at(dir, addr).state()?.address().context("the throwaway network has no address")?.trim_end_matches('/').to_string();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin)?;
    let helper = bin.join(one::git_wire::helper());
    let _ = std::fs::remove_file(&helper);
    std::os::unix::fs::symlink(network::program(dir)?, &helper)?;
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
    node_at(dir, &addr).post("/v1/settings", json!({ "name": one::database::scopes::CLOUDFLARE_NODE, "value": key }))?;
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
            form("throwaway sign-in", "sign its CLI in as its user, with no browser".into()),
            form("throwaway git <args>", format!("git, reaching its repos at its address: {CLI}-dev throwaway git clone <address>/repos/<owner>/<repo>.git, then {CLI}-dev throwaway git push in the clone")),
            form("throwaway cloudflare node deploy", "run its Cloudflare node beside its Workers, as Cloudflare would".into()),
            form("throwaway client <kind>", "a signed-in client of this checkout's build: web, linux, mac, iphone, android or windows".into()),
            form("throwaway client <kind> --as sam", "the same, as someone else, invited by you with Add Someone's link".into()),
            form("throwaway client cli <command>", format!("a command from a computer that's only a device of its user's (no node), asking their nodes over iroh in run priority's order, as every client does: --device cloudflare asks the Cloudflare node, once deployed")),
            form("throwaway stop", format!("stop and delete it, and give back its devices; {CLI}-dev throwaway then starts a new one")),
        ],
        run: |_| Ok(false),
    }
}
