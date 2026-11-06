//! `throwaway`: a throwaway network, a network of its own, made as `create` makes every new one, by a node beside this machine's own with its own
//! home and ports, so it touches nothing of the person's (`one::mode`). There's one for each checkout it's run in, from any folder in it, so threads working in different ones each have their
//! own. Its Workers run beside it, and its sign-in lets its first person in with no passkey (cloudflare::Settings::throwaway_person), so a
//! client of this checkout's build, on a device borrowed for it (`throwaway client`), signs in with nobody there; others join
//! as anyone does, by an invite link the first makes, and again with no passkey (`throwaway client <kind> --as <name>`). It
//! keeps its data until `throwaway stop`: left alone it stops, and `throwaway` starts it again as it was, as it does when this build
//! is newer than the one it runs, unless a client session (`throwaway client`) is running, which a new build must not break:
//! then at the next command with none. Starting it again leaves its clients' browsers alone.


use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use one::config::CLI;
use one::mode::{Mode, Throwaway};
use serde_json::json;

mod clients;
mod devices;
mod network;
mod rebuild;

use network::{older_build, no_client_running, running, start, stop, stop_some, throwaway_agent};

/// The throwaway network for the checkout this build is from (`<checkout>/target/<profile>/`), else for the checkout the
/// current folder is in, wherever in it, as git finds it, beside the downloads every throwaway network shares: → its folder,
/// and the checkout. So a checkout's own build finds its throwaway network from any folder, as an MCP client starts it, and
/// from a folder in no checkout there is none: nothing starts a network for a folder that isn't one.
fn folder() -> Option<(PathBuf, PathBuf)> {
    let base = PathBuf::from(format!("{}-throwaway", one::home().display()));
    let is_checkout = |top: &Path| top.join(".git").exists() && top.join("Cargo.toml").exists() && top.join("ARCHITECTURE.md").exists();
    let built = std::env::current_exe().ok().and_then(|exe| exe.canonicalize().ok()).and_then(|exe| Some(exe.parent()?.parent()?.parent()?.to_path_buf())).filter(|top| is_checkout(top));
    let top = || std::process::Command::new("git").args(["rev-parse", "--show-toplevel"]).output().ok().filter(|o| o.status.success()).map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())).filter(|top| is_checkout(top));
    let checkout = built.or_else(top)?;
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(checkout.to_string_lossy().as_bytes()).into();
    let hash = one::keys::hex(&hash);
    let name = checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Some((base.join(format!("{name}-{}", &hash[..6])), checkout))
}

/// A command typed after `throwaway`. The first time, in the real network's CLI, it makes sure the throwaway network runs and, but for
/// the commands that manage it, runs the command again as one of its programs (`Throwaway::tell`), where `run` takes it from `inside`.
pub fn run(rest: &[String]) -> Result<()> {
    if let Mode::Throwaway(network) = one::mode::here() {
        return inside(network, rest);
    }
    // Help, and nothing started: throwaway's own, or a built-in command's. An app's tool's needs the throwaway network's apps.
    let help = rest.iter().any(|a| a == "--help" || a == "-h");
    match rest.first().map(String::as_str) {
        Some("--help" | "-h" | "help") | Some("stop" | "client" | "sign-in") if help || rest[0] == "help" => {
            println!("{}", usage());
            return Ok(());
        }
        Some(cmd) if help && crate::command_words().iter().any(|c| c == cmd) => return crate::run(rest.to_vec()),
        _ => {}
    }
    let Some((dir, checkout)) = folder() else {
        bail!("{CLI} throwaway is for a checkout of The One App, and this folder isn't in one: run it in the checkout, or any folder in it")
    };
    match rest.first().map(String::as_str) {
        Some("stop") => {
            devices::give_back(&dir, &checkout);
            stop(&dir, true)?;
            println!("Stopped and deleted this checkout's throwaway network, and gave back its devices. {CLI} throwaway starts a new one.");
            return Ok(());
        }
        Some(cmd @ ("update" | "release" | "publish" | "throwaway")) => bail!("{CLI} {cmd} isn't for the throwaway network"),
        Some("cloudflare") if rest.get(1).map(String::as_str) == Some("serve") => bail!("the throwaway network runs its own Workers"),
        _ => {}
    }
    if rest.first().map(String::as_str) == Some("client") && rest.get(1).map(String::as_str) == Some("web") {
        rebuild::web_client_built(&checkout, rest)?;
    }
    if rest.iter().map(String::as_str).eq(["cloudflare", "node", "deploy"]) {
        rebuild::cloudflare_node_built(&checkout, rest)?;
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
    if rest.iter().map(String::as_str).eq(["cloudflare", "node", "deploy"]) {
        return cloudflare_node(&dir);
    }
    if rest.first().map(String::as_str) == Some("client") {
        let settings = one::cloudflare::Settings::load_in(&dir)?;
        return clients::client(&dir, &checkout, &settings, &rest[1..]);
    }
    if rest.is_empty() {
        return say_where(&dir, &addr);
    }
    // As one of its programs: its home, so the CLI asks its node with its token, as nobody signed in, and its Cloudflare
    // settings are its own Workers'; and its mode, so nothing of this machine's own network's is touched.
    use std::os::unix::process::CommandExt;
    let mut again = std::process::Command::new(std::env::current_exe()?);
    Throwaway::tell(&mut again, &dir, &addr);
    Err(again.arg("throwaway").args(rest).exec().into())
}

/// A command, run as a program of the throwaway network: `sign-in` and `git`, which are its own, or any other command, which
/// this program's other code runs against the network it's a program of.
fn inside(network: &Throwaway, rest: &[String]) -> Result<()> {
    match rest.first().map(String::as_str) {
        Some("sign-in") => sign_in(),
        Some("git") => git(network.home(), &rest[1..]),
        _ => crate::run(rest.to_vec()),
    }
}

/// `sign-in`, as the throwaway network's person, with no passkey: its sign-in asks for none.
fn sign_in() -> Result<()> {
    let address = crate::node().get("/v1/state")?["network"]["settings"]["address"].as_str().context("the throwaway network has no address")?.to_string();
    let started = one::sign_in::start(&address, None)?;
    one::sign_in::approve_on_throwaway(&address, &started.url, None)?;
    one::sign_in::finish(started)?;
    let me = crate::node().get("/v1/state")?["user"]["name"].as_str().unwrap_or("you").to_string();
    println!("Signed in as {me}. {CLI} throwaway and {CLI} throwaway mcp act as them until {CLI} throwaway sign-out.");
    Ok(())
}

/// What the bare `throwaway` says: that it runs, and where.
fn say_where(dir: &Path, addr: &str) -> Result<()> {
    let state = crate::request(throwaway_agent(dir).get(format!("http://{addr}/v1/state")).call())?;
    let me = state["user"]["name"].as_str().or(state["me"]["name"].as_str()).unwrap_or("you");
    let address = state["network"]["settings"]["address"].as_str().unwrap_or("");
    println!(
        "This checkout's throwaway network is running on this machine: a new network, Throwaway, with only what every new network
starts with, and {me} as its admin, who signs in at {address} with no passkey. Its folder is {}, with its
recovery key; its Workers' settings are in {}.

{CLI} throwaway --help lists its commands.",
        dir.display(),
        dir.join("cloudflare").join("settings.json").display()
    );
    Ok(())
}

/// `git`, told that the throwaway network's repos (`<its address>/repos/…`) are reached through this build's remote helper, as
/// git is on a computer signed in to any network: this build's helper is linked in a folder of the throwaway network's, put
/// first on git's PATH (the one the real network's CLI links is another build, reaching another node).
fn git(dir: &Path, args: &[String]) -> Result<()> {
    let address = crate::node().get("/v1/state")?["network"]["settings"]["address"].as_str().context("the throwaway network has no address")?.trim_end_matches('/').to_string();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin)?;
    let link = bin.join(crate::git_remote::helper());
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(std::env::current_exe()?.canonicalize()?, &link)?;
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())))?;
    let config: usize = std::env::var("GIT_CONFIG_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let status = std::process::Command::new("git")
        .args(args)
        .env("PATH", path)
        .env("GIT_CONFIG_COUNT", (config + 1).to_string())
        .env(format!("GIT_CONFIG_KEY_{config}"), format!("url.{CLI}::{address}/repos/.insteadOf"))
        .env(format!("GIT_CONFIG_VALUE_{config}"), format!("{address}/repos/"))
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
    anyhow::ensure!(one::cloudflare::node::modules().is_ok(), "this build has no Cloudflare node: run this checkout's own build, target/debug/{CLI} throwaway cloudflare node deploy, which builds it");
    let mut settings = one::cloudflare::Settings::load_in(dir)?;
    if !settings.cloudflare_node {
        settings.cloudflare_node = true;
        settings.save()?;
        stop_some(dir, false, false)?;
        start(dir)?;
    }
    let addr = running(dir).context("the throwaway network didn't start again")?;
    let key = settings.cloudflare_node_endpoint().context("the throwaway network's Workers have no Cloudflare node key")?;
    crate::request(throwaway_agent(dir).post(format!("http://{addr}/v1/settings")).send_json(json!({ "name": one::database::scopes::CLOUDFLARE_NODE, "value": key })))?;
    println!("The throwaway network's Cloudflare node runs beside its Workers, its endpoint {}.", &key[..8]);
    Ok(())
}

/// What `throwaway` takes.
fn usage() -> String {
    // The MCP command as one that finds this throwaway network from any folder: this build, if it's a checkout's own.
    let mcp = match folder() {
        Some((_, checkout)) => match std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).filter(|e| e.starts_with(&checkout)) {
            Some(exe) => format!("{} throwaway mcp", exe.display()),
            None => format!("sh -c 'cd {} && exec {CLI} throwaway mcp'", checkout.display()),
        },
        None => format!("sh -c 'cd <checkout> && exec {CLI} throwaway mcp'"),
    };
    format!(
        "Usage:
  {CLI} throwaway                  start this checkout's throwaway network, if it isn't running (or runs an older build), and say where it is
  {CLI} throwaway <command>        any command, in it: {CLI} throwaway status, {CLI} throwaway apps, {CLI} throwaway repos list
  {CLI} throwaway mcp              its MCP server, for an agent: claude mcp add {CLI}-throwaway -- {mcp}
  {CLI} throwaway sign-in          sign its CLI in as its person, with no browser
  {CLI} throwaway git <args>       git, reaching its repos at its address: {CLI} throwaway git clone <address>/repos/<owner>/<repo>.git, then {CLI} throwaway git push in the clone
  {CLI} throwaway cloudflare node deploy    run its Cloudflare node beside its Workers, as Cloudflare would
  {CLI} throwaway client <kind>    a signed-in client of this checkout's build: web, linux, mac, iphone, android or windows
  {CLI} throwaway client <kind> --as sam    the same, as someone else, invited by you with Add Someone's link
  {CLI} throwaway client cli <command>    a command from a computer that's only a device of its person's (no node), asking
                                   their nodes over iroh in run priority's order, as every client does: the Cloudflare
                                   node, once deployed, with {CLI} throwaway settings priority '{{\"cloudflare\":1}}';
                                   --build <program> first runs another build of {CLI} (an older release's) as that device
  {CLI} throwaway stop             stop and delete it, and give back its devices; {CLI} throwaway then starts a new one"
    )
}

/// `throwaway`'s help: it runs before the words after it are read, as their own command (`run`).
pub fn command() -> one::cli::Command {
    one::cli::Command {
        help: vec![
            ("throwaway [<command>]".into(), format!("a throwaway network for this checkout, on this machine beside yours, with only what every new one starts with: to look around and test in. {CLI} throwaway <command> runs a command in it ({CLI} throwaway mcp for an agent), {CLI} throwaway client web, linux, mac, iphone, android or windows gives a client of this checkout's build signed in to it, and {CLI} throwaway stop deletes it")),
        ],
        run: |_| Ok(false),
    }
}
