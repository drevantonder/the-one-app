//! `throwaway`: a throwaway network, a network of its own, made as `create` makes every new one, by a node beside this machine's own with its own
//! home and ports, so it touches nothing of the person's (src/agents.rs leaves the harnesses alone for a node given a
//! home). There's one for each checkout it's run in, from any folder in it, so threads working in different ones each have their
//! own. Its Workers run beside it, and its sign-in lets its first person in with no passkey (cloudflare::Settings::throwaway_person), so a
//! client of this checkout's build, on a device borrowed for it (`throwaway client`), signs in with nobody there; others join
//! as anyone does, by an invite link the first makes, and again with no passkey (`throwaway client <kind> --as <name>`). It
//! keeps its data until `throwaway stop`: left alone it stops, and `throwaway` starts it again as it was, as it does when this build
//! is newer than the one it runs, unless a client session (`throwaway client`) is running, which a new build must not break:
//! then at the next command with none. Starting it again leaves its clients' browsers alone.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use one::config::{CLI, NAME};
use serde_json::{Value, json};

use crate::{agent, get, post};

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

fn addr_var() -> String {
    format!("{}_ADDR", CLI.to_uppercase())
}

/// The throwaway network's node's address, while its node runs and answers: an answer from the address alone could be
/// another checkout's throwaway network, which took the port while this one was stopped.
fn running(dir: &Path) -> Option<String> {
    let addr = std::fs::read_to_string(dir.join("addr")).ok()?.trim().to_string();
    let node: i32 = std::fs::read_to_string(dir.join("pid")).ok()?.trim().parse().ok()?;
    programs(dir, false).contains(&node).then_some(())?;
    agent().get(format!("http://{addr}/v1/state")).call().is_ok().then_some(addr)
}

/// Whether the throwaway network's node started before this build was made: it runs the build it started as, Workers and screens
/// included, so a new build (`cargo build`) is picked up by starting it again, which keeps its data.
fn older_build(dir: &Path) -> bool {
    let modified = |path: &Path| path.metadata().and_then(|m| m.modified()).ok();
    match (std::env::current_exe().ok().and_then(|exe| modified(&exe)), modified(&dir.join("pid"))) {
        (Some(built), Some(started)) => built > started,
        _ => false,
    }
}

/// Whether no client session (`throwaway client`, which holds `client.lock` shared while it builds and signs in) is running:
/// only then may a new build start the throwaway network again, as that drops what a client reaches it by.
fn no_client_running(dir: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(file) = std::fs::File::options().create(true).append(true).open(dir.join("client.lock")) else { return true };
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

/// Holds `client.lock` shared until the returned file is dropped, or the process ends: a client session is running.
fn client_session(dir: &Path) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::options().create(true).append(true).open(dir.join("client.lock")).ok()?;
    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH) } == 0).then_some(file)
}

pub fn run(rest: &[String]) -> Result<()> {
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
            give_back(&dir, &checkout);
            stop(&dir, true)?;
            println!("Stopped and deleted this checkout's throwaway network, and gave back its devices. {CLI} throwaway starts a new one.");
            return Ok(());
        }
        Some(cmd @ ("update" | "release" | "publish" | "throwaway")) => bail!("{CLI} {cmd} isn't for the throwaway network"),
        Some("cloudflare") if rest.get(1).map(String::as_str) == Some("serve") => bail!("the throwaway network runs its own Workers"),
        _ => {}
    }
    if rest.first().map(String::as_str) == Some("client") && rest.get(1).map(String::as_str) == Some("web") {
        web_client_built(&checkout, rest)?;
    }
    if rest.iter().map(String::as_str).eq(["cloudflare", "node", "deploy"]) {
        cloudflare_node_built(&checkout, rest)?;
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
    let cloudflare = dir.join("cloudflare").join("settings.json");
    if rest.iter().map(String::as_str).eq(["cloudflare", "node", "deploy"]) {
        return cloudflare_node(&dir);
    }
    if rest.first().map(String::as_str) == Some("client") {
        let settings = load_cloudflare(&dir)?;
        return client(&dir, &checkout, &settings, &rest[1..]);
    }
    // Before any thread starts: the CLI has none. Its home, so the CLI asks its node with its token, as nobody signed in,
    // and its Cloudflare settings are its own Workers'.
    unsafe { std::env::set_var(addr_var(), &addr) };
    unsafe { std::env::set_var(format!("{}_HOME", CLI.to_uppercase()), &dir) };
    if rest.first().map(String::as_str) == Some("sign-in") {
        // As its person, with no passkey: the throwaway network's sign-in asks for none.
        let address = get("/v1/state")?["network"]["settings"]["address"].as_str().context("the throwaway network has no address")?.to_string();
        let started = one::sign_in::start(&address, None)?;
        one::sign_in::approve_on_throwaway(&address, &started.url, None)?;
        one::sign_in::finish(started)?;
        let me = get("/v1/state")?["user"]["name"].as_str().unwrap_or("you").to_string();
        println!("Signed in as {me}. {CLI} throwaway and {CLI} throwaway mcp act as them until {CLI} throwaway sign-out.");
        return Ok(());
    }
    if rest.first().map(String::as_str) == Some("git") {
        return git(&dir, &rest[1..]);
    }
    if !rest.is_empty() {
        return crate::run(rest.to_vec());
    }
    let state = get("/v1/state")?;
    let me = state["user"]["name"].as_str().or(state["me"]["name"].as_str()).unwrap_or("you");
    let address = state["network"]["settings"]["address"].as_str().unwrap_or("");
    println!(
        "This checkout's throwaway network is running on this machine: a new network, Throwaway, with only what every new network
starts with, and {me} as its admin, who signs in at {address} with no passkey. Its folder is {}, with its
recovery key; its Workers' settings are in {}.

{CLI} throwaway --help lists its commands.",
        dir.display(),
        cloudflare.display()
    );
    Ok(())
}

/// `git`, told that the throwaway network's repos (`<its address>/repos/…`) are reached through this build's remote helper, as
/// git is on a computer signed in to any network: this build's helper is linked in a folder of the throwaway network's, put
/// first on git's PATH (the one the real network's CLI links is another build, reaching another node).
fn git(dir: &Path, args: &[String]) -> Result<()> {
    let address = get("/v1/state")?["network"]["settings"]["address"].as_str().context("the throwaway network has no address")?.trim_end_matches('/').to_string();
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
    let mut settings = load_cloudflare(dir)?;
    if !settings.cloudflare_node {
        settings.cloudflare_node = true;
        with_home(dir, || settings.save())?;
        stop_some(dir, false, false)?;
        start(dir)?;
    }
    let addr = running(dir).context("the throwaway network didn't start again")?;
    let key = settings.cloudflare_node_endpoint().context("the throwaway network's Workers have no Cloudflare node key")?;
    crate::request(throwaway_agent(dir).post(format!("http://{addr}/v1/settings")).send_json(json!({ "name": "serverless", "value": key })))?;
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

/// The throwaway network's Cloudflare settings, with their secrets, read as `cloudflare deploy` reads them.
fn load_cloudflare(dir: &Path) -> Result<one::cloudflare::Settings> {
    with_home(dir, one::cloudflare::Settings::load)
}

/// `f`, with this CLI's home the throwaway network's.
fn with_home<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    let var = format!("{}_HOME", CLI.to_uppercase());
    let was = std::env::var_os(&var);
    unsafe { std::env::set_var(&var, dir) };
    let out = f();
    match was {
        Some(was) => unsafe { std::env::set_var(&var, was) },
        None => unsafe { std::env::remove_var(&var) },
    }
    out
}

/// `f`, with this CLI reaching the node at `addr`.
fn with_addr<T>(addr: &str, f: impl FnOnce() -> T) -> T {
    let var = addr_var();
    let was = std::env::var_os(&var);
    unsafe { std::env::set_var(&var, addr) };
    let out = f();
    match was {
        Some(was) => unsafe { std::env::set_var(&var, was) },
        None => unsafe { std::env::remove_var(&var) },
    }
    out
}

/// Starts the throwaway network's node and Workers: as it was, if it stopped by itself, or a new one. → the node's address.
fn start(dir: &Path) -> Result<String> {
    let kept = dir.join("cloudflare").join("settings.json").exists() && dir.join("addr").exists();
    if !kept {
        return start_new(dir);
    }
    eprintln!("Starting this checkout's throwaway network again…");
    stop_some(dir, false, false)?;
    let was = std::fs::read_to_string(dir.join("addr"))?.trim().to_string();
    // Its address, held while its Workers start (seconds), so another checkout's throwaway network starting meanwhile
    // can't take it: it keeps the one it had, else the system gives a free one, which the node's address file then says.
    let (held, addr) = reserve(&was).or_else(|_| reserve("127.0.0.1:0"))?;
    // Its Workers first, so its node's relay finds them the first time it asks: found later, it waits out a backoff of
    // about half a minute, while the Cloudflare node says none of the network's devices is on.
    spawn_cloudflare(dir, &load_cloudflare(dir)?, &addr)?;
    drop(held);
    spawn_node(dir, Some(&addr))
}

/// A TCP port held for the node, which isn't started yet: bound, not listening, so what asks for it is refused at once
/// where a listener would leave it waiting. → what holds it, until dropped, and its address.
fn reserve(addr: &str) -> Result<(std::os::fd::OwnedFd, String)> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let addr: std::net::SocketAddrV4 = addr.parse().context("the throwaway network's address")?;
    // macOS has no SOCK_CLOEXEC: it's set after, before anything here starts a program.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    anyhow::ensure!(fd >= 0, "no socket: {}", std::io::Error::last_os_error());
    let held = unsafe { OwnedFd::from_raw_fd(fd) };
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    // Zeroed, then filled in: macOS's has a length field too (sin_len), which zero is fine for.
    let mut sin: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sin.sin_family = libc::AF_INET as _;
    sin.sin_port = addr.port().to_be();
    sin.sin_addr = libc::in_addr { s_addr: u32::from_ne_bytes(addr.ip().octets()) };
    let bound = unsafe { libc::bind(fd, (&sin as *const libc::sockaddr_in).cast(), std::mem::size_of::<libc::sockaddr_in>() as _) };
    anyhow::ensure!(bound == 0, "{addr} is taken: {}", std::io::Error::last_os_error());
    // With port 0 the system picked one: which.
    let mut got: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let named = unsafe { libc::getsockname(fd, (&mut got as *mut libc::sockaddr_in).cast(), &mut len) };
    anyhow::ensure!(named == 0, "{}", std::io::Error::last_os_error());
    Ok((held, format!("{}:{}", addr.ip(), u16::from_be(got.sin_port))))
}

/// A new throwaway network, and its Workers.
fn start_new(dir: &Path) -> Result<String> {
    eprintln!("Starting a new throwaway network for this checkout…");
    stop(dir, true)?;
    // Called what the person is called in their own network, if this machine is in one.
    let quick: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).middleware(one::api::as_this_machine).build().into();
    let mine = quick.get(format!("http://{}/v1/state", crate::addr())).call().ok().and_then(|mut r| r.body_mut().read_json::<Value>().ok());
    let me = mine.and_then(|s| s["user"]["name"].as_str().or(s["me"]["name"].as_str()).map(String::from));
    let me = me.or_else(|| std::env::var("USER").ok()).unwrap_or_else(|| "You".into());
    std::fs::create_dir_all(dir)?;
    let bucket = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string();
    std::fs::write(dir.join("bucket-addr"), &bucket)?;
    // A free port the system picks, as for everything else here: never a fixed one every checkout's network would
    // want first, which another's would hold when this one starts again.
    let addr = spawn_node(dir, None)?;
    let agent = || throwaway_agent(dir);
    let on = |path: &str| format!("http://{addr}{path}");
    let created = crate::request(agent().post(on("/v1/create")).send_json(json!({ "name": "Throwaway", "user_name": me })))?;
    let recovery = created["recovery"].as_str().context("the throwaway network gave no recovery key")?;
    write_private(&dir.join("recovery-key"), &format!("{recovery}\n"))?;
    let state = crate::request(agent().get(on("/v1/state")).call())?;
    let person = (state["user"]["id"].as_str().context("the throwaway network has no person")?.to_string(), me);
    let settings = one::cloudflare::Settings::for_throwaway(&bucket, person)?;
    with_home(dir, || settings.save())?;
    // Read back as `cloudflare deploy` reads it, with its secrets.
    let settings = load_cloudflare(dir)?;
    spawn_cloudflare(dir, &settings, &addr)?;
    crate::request(agent().post(on("/v1/settings")).send_json(json!({ "name": "address", "value": &settings.address })))?;
    // Its sign-in signs its own ID tokens, as a provider would, for this client: the one its people sign in with.
    let clients = format!("{}/api/auth one-throwaway", settings.address);
    crate::request(agent().post(on("/v1/settings")).send_json(json!({ "name": one::sign_in_proof::CLIENTS, "value": clients })))?;
    // Its data moves to the throwaway bucket, which its node reaches only through its URL signer from then on.
    let moved = crate::request(agent().post(on("/v1/storage")).send_json(json!({ "endpoint": format!("http://{bucket}"), "bucket": "throwaway" })))?;
    anyhow::ensure!(moved["kept"] == true, "the throwaway network's node couldn't reach its bucket through the URL signer: {moved}");
    let started = std::time::Instant::now();
    while crate::request(agent().get(on("/v1/storage")).call())?["kind"] != "s3" {
        anyhow::ensure!(started.elapsed() < Duration::from_secs(30), "the throwaway network's node didn't move to its bucket: node.log says why");
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(addr)
}

/// Starts a program of the throwaway network's in a process group of its own, which stopping it stops (`pids`), logging to
/// `log` in its folder. A client's browser is `client`: starting the network again leaves it running.
fn spawn(dir: &Path, command: &mut std::process::Command, log: &str, client: bool) -> Result<u32> {
    use std::os::unix::process::CommandExt;
    let log = std::fs::File::create(dir.join(log))?;
    let child = command.env(format!("{}_HOME", CLI.to_uppercase()), dir).stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log).process_group(0).spawn()?;
    let mut pids = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
    pids += &format!("{} {}{}\n", child.id(), started(child.id() as i32).unwrap_or_default(), if client { " client" } else { "" });
    std::fs::write(dir.join("pids"), pids)?;
    Ok(child.id())
}

/// Starts the throwaway network's node, at `addr` if it's given and free, and gives its address once it answers.
fn spawn_node(dir: &Path, addr: Option<&str>) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    // What every throwaway network downloads or builds, shared.
    let shared = dir.parent().unwrap().join("cache");
    std::fs::create_dir_all(&shared)?;
    if !dir.join("cache").exists() {
        std::os::unix::fs::symlink(&shared, dir.join("cache"))?;
    }
    let addr = match addr {
        Some(addr) => addr.to_string(),
        None => std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string(),
    };
    let pid = spawn(
        dir,
        one::oned::command(&["--throwaway"])?.env(addr_var(), &addr),
        "node.log",
        false,
    )?;
    std::fs::write(dir.join("pid"), pid.to_string())?;
    std::fs::write(dir.join("addr"), &addr)?;
    wait_for(&throwaway_agent(dir), &format!("http://{addr}/v1/state"), &dir.join("node.log"))?;
    Ok(addr)
}

/// A client asking the throwaway network's node as one of this machine's programs: with the token in its home.
fn throwaway_agent(dir: &Path) -> ureq::Agent {
    let dir = dir.to_path_buf();
    let with_token = move |mut req: ureq::http::Request<ureq::SendBody>, next: ureq::middleware::MiddlewareNext| {
        if let Some(token) = one::api::local_token(&dir).and_then(|t| t.parse().ok()) {
            req.headers_mut().insert(one::api::LOCAL_TOKEN_HEADER, token);
        }
        next.handle(req)
    };
    ureq::Agent::config_builder().http_status_as_error(false).middleware(with_token).build().into()
}

/// Starts the throwaway network's Workers, beside its node at `node`, and waits for its address to answer.
fn spawn_cloudflare(dir: &Path, settings: &one::cloudflare::Settings, node: &str) -> Result<()> {
    for port in settings.ports {
        anyhow::ensure!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "the throwaway network's Workers' port {port} was taken by another program while it was stopped: {CLI} throwaway stop, then {CLI} throwaway for a new one on free ports");
    }
    // A sign-in kept for this node's address by an earlier throwaway network, whose Workers are gone, would be sent to this
    // node, which refuses it: its relay would find no devices, and the CLI says the key isn't published.
    with_addr(node, || {
        if one::sign_in::kept().is_some_and(|kept| kept.address != settings.address) {
            let _ = std::fs::remove_file(one::sign_in::path());
        }
    });
    // Beside its node, which it asks for the network's devices, for its relay to take only them.
    spawn(dir, std::process::Command::new(std::env::current_exe()?).args(["cloudflare", "serve"]).env(addr_var(), node), "cloudflare.log", false)?;
    wait_for(&agent(), &format!("{}/.well-known/oauth-authorization-server", settings.address), &dir.join("cloudflare.log"))
}

fn wait_for(agent: &ureq::Agent, url: &str, log: &Path) -> Result<()> {
    let start = Instant::now();
    while !agent.get(url).call().is_ok_and(|r| r.status().is_success()) {
        anyhow::ensure!(start.elapsed() < Duration::from_secs(120), "the throwaway network didn't start: {}", log.display());
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// When a process started, where this machine says (Linux's /proc): in clock ticks since it booted.
fn started(pid: i32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // After the command's name, in parentheses, which may hold spaces: the 22nd field.
    stat.rsplit_once(')')?.1.split_whitespace().nth(19).map(String::from)
}

/// The throwaway network's programs that still run, by the process groups they lead (`spawn`): only those that started when
/// they did, where this machine says, as another process may have taken a pid since. With `clients` false, not the
/// clients' browsers.
fn programs(dir: &Path, clients: bool) -> Vec<i32> {
    let pids = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
    let ours = |pid: i32, at: &str| at.is_empty() || started(pid).as_deref() == Some(at);
    pids.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next().unwrap_or("").to_string(), fields.next() == Some("client")))
        })
        .filter(|(pid, at, client)| ours(*pid, at) && (clients || !client))
        .map(|(pid, _, _)| pid)
        .collect()
}

/// Stops the throwaway network's programs, if they run, and with `delete`, deletes all of it but what it downloaded or built.
pub fn stop(dir: &Path, delete: bool) -> Result<()> {
    stop_some(dir, delete, true)
}

/// `stop`, and with `clients` false, leaving the clients' browsers running, as starting the network again does.
fn stop_some(dir: &Path, delete: bool, clients: bool) -> Result<()> {
    let pids = programs(dir, clients);
    for pid in &pids {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
    }
    let start = Instant::now();
    while pids.iter().any(|pid| unsafe { libc::kill(*pid, 0) } == 0) && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
    }
    for pid in &pids {
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    // What still runs stays listed: its browsers.
    let kept: String = std::fs::read_to_string(dir.join("pids")).unwrap_or_default().lines().filter(|l| !clients && l.ends_with(" client")).map(|l| format!("{l}\n")).collect();
    if kept.is_empty() {
        let _ = std::fs::remove_file(dir.join("pids"));
    } else {
        std::fs::write(dir.join("pids"), kept)?;
    }
    let _ = std::fs::remove_file(dir.join("pid"));
    // The Mac app it started there (`client mac`), and the simulator it made there for the iPhone app (`client iphone`).
    if clients && let Ok(host) = std::fs::read_to_string(dir.join("mac")) {
        let job = format!("gui/$(id -u)/app.{CLI}.mac.throwaway");
        let _ = std::process::Command::new("ssh").args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host.trim(), &format!("launchctl bootout {job}")]).output();
        let _ = std::fs::remove_file(dir.join("mac"));
    }
    if clients && let Ok(host) = std::fs::read_to_string(dir.join("iphone")) {
        let sim = format!("'{CLI} throwaway'");
        let _ = std::process::Command::new("ssh").args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host.trim(), &format!("xcrun simctl shutdown {sim}; xcrun simctl delete {sim}")]).output();
        let _ = std::fs::remove_file(dir.join("iphone"));
    }
    if delete {
        // Its Linux devices, borrowed or spare, are its own containers on the machine's runtime, outside its folder.
        // By Agent Devices' own program, which knows where they are.
        let program = one::oned::program("agent-devices");
        if dir.join("agent-devices").exists() && program.exists() {
            match std::process::Command::new(&program).arg("delete-all").arg(dir.join("agent-devices")).output() {
                Ok(out) if out.status.success() => {}
                Ok(out) => eprintln!("Couldn't delete its Linux devices: {}", String::from_utf8_lossy(&out.stderr).trim()),
                Err(err) => eprintln!("Couldn't delete its Linux devices: {err}"),
            }
        }
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if entry.file_name() != "cache" {
                let path = entry.path();
                if path.is_dir() { std::fs::remove_dir_all(&path)? } else { std::fs::remove_file(&path)? }
            }
        }
    }
    Ok(())
}

/// Stops what the throwaway network in `home` started besides its node, which then stops itself: its Workers and browser.
pub fn stop_others(home: &Path) {
    let me = std::process::id() as i32;
    for pid in programs(home, true) {
        if pid != me {
            unsafe { libc::kill(-pid, libc::SIGTERM) };
        }
    }
    let _ = std::fs::remove_file(home.join("pids"));
}

/// The throwaway network's bucket, at `bucket-addr` in its folder: the folder its node made the network in, served S3's way
/// for the URLs its URL signer presigns (read, HEAD, list, create only if absent, replace while unchanged,
/// delete), signatures unchecked, as it's only this machine's and only a throwaway network's.
pub async fn bucket(home: PathBuf) -> Result<()> {
    use axum::http::{HeaderMap, Method, StatusCode, Uri};
    let Ok(at) = std::fs::read_to_string(home.join("bucket-addr")) else { return Ok(()) };
    // A node taking over from another waits for the one before to let the port go.
    let mut tries = 0;
    let listener = loop {
        match tokio::net::TcpListener::bind(at.trim()).await {
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse && tries < 600 => tries += 1,
            bound => break bound?,
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let place = one::bucket::Place::Folder(home.join("network").join("bucket"));
    let answer = move |method: Method, uri: Uri, headers: HeaderMap, body: axum::body::Bytes| {
        let place = place.clone();
        async move {
            if method == Method::GET && matches!(uri.path(), "/throwaway" | "/throwaway/") {
                let query: Vec<(String, String)> = uri.query().unwrap_or("").split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (decode(k), decode(v))).collect();
                let get = |k: &str| query.iter().find(|(q, _)| q == k).map(|(_, v)| v.clone());
                let (prefix, delimit) = (get("prefix").unwrap_or_default(), get("delimiter").is_some());
                let dir = home.join("network").join("bucket");
                return match tokio::task::spawn_blocking(move || list(&dir, &prefix, delimit)).await {
                    Ok(Ok(xml)) => (StatusCode::OK, HeaderMap::new(), xml.into_bytes()),
                    _ => (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new(), Vec::new()),
                };
            }
            let name = decode(uri.path().strip_prefix("/throwaway/").unwrap_or(""));
            if name.is_empty() || name.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
                return (StatusCode::BAD_REQUEST, HeaderMap::new(), Vec::new());
            }
            let header = move |h: &str| headers.get(h).and_then(|v| v.to_str().ok()).map(|v| v.trim_matches('"').to_string());
            let done = |made: bool| if made { StatusCode::OK } else { StatusCode::PRECONDITION_FAILED };
            let answered = tokio::task::spawn_blocking(move || -> Result<(StatusCode, HeaderMap, Vec<u8>)> {
                Ok(match method {
                    Method::GET | Method::HEAD => match place.get_object(&name)? {
                        Some(object) => {
                            let tag = [("etag".parse().unwrap(), format!("\"{}\"", object.tag).parse()?)].into_iter().collect();
                            (StatusCode::OK, tag, if method == Method::HEAD { Vec::new() } else { object.bytes })
                        }
                        None => (StatusCode::NOT_FOUND, HeaderMap::new(), Vec::new()),
                    },
                    Method::PUT if header("if-none-match").as_deref() == Some("*") => (done(place.create(&name, &body)?), HeaderMap::new(), Vec::new()),
                    Method::PUT => (done(place.replace(&name, &body, header("if-match").as_deref())?), HeaderMap::new(), Vec::new()),
                    Method::DELETE => (place.delete(&name).map(|_| StatusCode::NO_CONTENT)?, HeaderMap::new(), Vec::new()),
                    _ => (StatusCode::METHOD_NOT_ALLOWED, HeaderMap::new(), Vec::new()),
                })
            });
            match answered.await {
                Ok(Ok(answer)) => answer,
                _ => (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new(), Vec::new()),
            }
        }
    };
    // Any size, as a real bucket takes: a service's build, a release's binary.
    axum::serve(listener, axum::Router::new().fallback(answer).layer(axum::extract::DefaultBodyLimit::disable())).await?;
    Ok(())
}

/// The names under `prefix` in the throwaway bucket's folder `dir`, as S3's ListObjectsV2 answers, in one page: one level
/// down with `delimit` (objects, and folders as common prefixes), else every object.
fn list(dir: &Path, prefix: &str, delimit: bool) -> Result<String> {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult><IsTruncated>false</IsTruncated>");
    let (folder, start) = prefix.rsplit_once('/').map_or(("", prefix), |(f, s)| (f, s));
    let place = one::bucket::Place::Folder(dir.to_path_buf());
    let names: Vec<String> = match delimit {
        true => place.list(folder)?.into_iter().filter(|n| n.starts_with(start)).map(|n| if folder.is_empty() { n } else { format!("{folder}/{n}") }).collect(),
        false => place.names()?.into_iter().filter(|n| n.starts_with(prefix)).collect(),
    };
    for name in names {
        let meta = std::fs::metadata(dir.join(&name))?;
        if meta.is_dir() {
            xml += &format!("<CommonPrefixes><Prefix>{}/</Prefix></CommonPrefixes>", esc(&name));
            continue;
        }
        let written = jiff::Timestamp::try_from(meta.modified()?)?;
        xml += &format!("<Contents><Key>{}</Key><LastModified>{written}</LastModified><Size>{}</Size></Contents>", esc(&name), meta.len());
    }
    Ok(xml + "</ListBucketResult>")
}

/// A URL's path, percent-decoded.
fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    std::io::Write::write_all(&mut file, text.as_bytes())?;
    Ok(())
}

// Clients.

/// The devices borrowed for the throwaway network, by kind: `{ "android": "<name>" }`.
fn devices(dir: &Path) -> serde_json::Map<String, Value> {
    std::fs::read(dir.join("devices.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// One of this machine's network's Agent Devices tools (not the throwaway network's, which has none). When the node
/// restarts during it (an update), or its service is still coming back, it's asked again once they answer.
fn agent_devices(tool: &str, input: Value) -> Result<Value> {
    Ok(again(|| post("/v1/call", json!({ "app": "agent-devices", "op": tool, "input": input, "client": "one throwaway" })))?["ok"].clone())
}

/// What `ask` gives, asking again for up to 3 minutes while this machine's node or a service of its restarts.
fn again(ask: impl Fn() -> Result<Value>) -> Result<Value> {
    let start = Instant::now();
    loop {
        match ask() {
            Err(err) if start.elapsed() < Duration::from_secs(180) && one::protocol::Coded::is(&err, one::protocol::code::RESTARTING) => {
                eprintln!("{NAME} is restarting here; asking again once it's back…");
                std::thread::sleep(Duration::from_secs(5));
            }
            out => return out,
        }
    }
}

/// A command run on a device, and what it said.
fn on_device(device: &str, command: &str) -> Result<String> {
    let out = agent_devices("run", json!({ "device": device, "command": command }))?;
    Ok(out.as_str().map(str::to_string).unwrap_or_else(|| out.to_string()))
}

/// The agent devices on this machine's network's devices, each with who borrowed it and what for (Computer's own list
/// leaves that out).
fn listed_devices() -> Value {
    agent_devices("_list", json!({})).map(|l| l["devices"].clone()).unwrap_or_default()
}

/// What a device borrowed for this checkout's throwaway network says it's for: `one throwaway client <kind>, for <checkout>`.
fn borrowed_for(checkout: &Path, kind: &str) -> String {
    format!("one throwaway client {kind}, for {}", checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

/// Gives back every device borrowed for the throwaway network: those devices.json names, and any Agent Devices lists as
/// borrowed for this checkout (what the file lost, or never got because the node restarted as it was lent).
fn give_back(dir: &Path, checkout: &Path) {
    let mut all: Vec<(String, String)> = devices(dir).into_iter().filter_map(|(kind, d)| Some((kind, d.as_str()?.to_string()))).collect();
    let ours = format!(", for {}", checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    for d in listed_devices().as_array().into_iter().flatten() {
        if let (Some(name), Some(what)) = (d["name"].as_str(), d["borrowed"]["for"].as_str())
            && what.starts_with("one throwaway client ")
            && what.ends_with(&ours)
            && !all.iter().any(|(_, n)| n == name)
        {
            all.push((what["one throwaway client ".len()..what.len() - ours.len()].to_string(), name.to_string()));
        }
    }
    for (kind, device) in all {
        if let Err(err) = agent_devices("give_back", json!({ "device": device })) {
            eprintln!("Couldn't give back the {kind} device {device}: {err:#}");
        }
    }
}

/// `throwaway client <kind> [--as <name>]`: a client of this checkout's build, signed in to the throwaway network as its first
/// person, or as someone else they invited, and how to see and use it. Again, it's the same client, with this
/// checkout's build as it is now.
fn client(dir: &Path, checkout: &Path, settings: &one::cloudflare::Settings, rest: &[String]) -> Result<()> {
    let started = Instant::now();
    // A session while it runs: a new build waits for the next command with none to start the throwaway network again.
    let _session = client_session(dir);
    let person = match rest.iter().position(|a| a == "--as") {
        Some(at) => Some(person_named(rest.get(at + 1).context("--as who? --as sam")?)?),
        None => None,
    };
    // As its first person is as nobody else.
    let first = settings.throwaway_person.as_ref().map(|(_, name)| name.to_lowercase());
    let person = person.filter(|name| Some(name.to_lowercase()) != first);
    match rest.first().map(String::as_str) {
        Some("web") => {
            // A browser reaches nodes over iroh, as the client's WebAssembly; a build without it would test the address.
            anyhow::ensure!(
                one::cloudflare::client_version().is_some(),
                "this build has no web client's WebAssembly: run this checkout's own build, target/debug/{CLI} throwaway client web, which builds it"
            );
            web(dir, settings, person.as_deref())?
        }
        Some("cli") => return cli(dir, settings, &rest[1..]),
        Some("linux") => {
            if let Some(name) = &person {
                joined(dir, settings, name)?;
            }
            linux(dir, checkout, settings, person.as_deref())?
        }
        Some(kind @ ("mac" | "iphone")) => {
            anyhow::ensure!(person.is_none(), "the {kind} client signs in only as the throwaway network's first person: use web, linux or android for someone else");
            apple(dir, checkout, settings, kind == "iphone")?
        }
        Some(kind @ ("android" | "windows")) => {
            if let Some(name) = &person {
                // They join where anyone does, in a browser: the throwaway network's own here.
                joined(dir, settings, name)?;
            }
            device_client(dir, checkout, settings, kind, person.as_deref())?
        }
        _ => bail!("which client? {CLI} throwaway client web, linux, mac, iphone, android, windows or cli, with --as <name> for someone else"),
    }
    println!("Ready in {}s.", started.elapsed().as_secs());
    Ok(())
}

/// `command`, run by this build's CLI as on a computer where no node runs: a client device of the throwaway network's
/// person, with a home of its own beside it (`cli/`), which asks their nodes over iroh as every client does, in run
/// priority's order (`client_device`). The first time it signs in with no passkey, and the throwaway network's machine
/// approves the code it shows. `--build <program>` runs another build of the CLI (an older release's, say) as that
/// device, with a home of its own (`cli-<its name>/`).
fn cli(dir: &Path, settings: &one::cloudflare::Settings, command: &[String]) -> Result<()> {
    let (program, command) = match command {
        [flag, program, rest @ ..] if flag == "--build" => (PathBuf::from(program).canonicalize().with_context(|| format!("no program {program}"))?, rest),
        _ => (std::env::current_exe()?, command),
    };
    let home = match program == std::env::current_exe()? {
        true => dir.join("cli"),
        false => dir.join(format!("cli-{}", program.file_name().unwrap_or_default().to_string_lossy())),
    };
    std::fs::create_dir_all(&home)?;
    // No node answers at its address, nor anything of this machine's in its home.
    let as_device = |args: &[String]| {
        let mut c = std::process::Command::new(&program);
        c.args(args).env(addr_var(), "127.0.0.1:1").env(format!("{}_HOME", CLI.to_uppercase()), &home).env("XDG_CONFIG_HOME", &home).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
        c
    };
    let joined = home.join("joined");
    if !joined.exists() {
        use std::io::BufRead;
        eprintln!("Signing in a CLI as a device of the throwaway network's person…");
        let mut child = as_device(&["sign-in".into(), "--address".into(), settings.address.clone()]).stdout(std::process::Stdio::piped()).spawn()?;
        for line in std::io::BufReader::new(child.stdout.take().context("no output from signing in")?).lines() {
            let line = line?;
            let said = line.trim();
            eprintln!("{line}");
            if said.starts_with(&settings.address) && said.contains("/sign-in") {
                one::sign_in::approve_on_throwaway(&settings.address, said, None)?;
            } else if is_code(said) {
                approve_code(dir, said)?;
            }
        }
        anyhow::ensure!(child.wait()?.success(), "the CLI didn't sign in as a device");
        std::fs::write(&joined, "")?;
    }
    if command.is_empty() {
        println!("A CLI that's only a device of the throwaway network's person is signed in: {CLI} throwaway client cli <command> runs one as it.");
        return Ok(());
    }
    let status = as_device(command).status()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// For `throwaway client web` from this checkout's own debug build: the web client's WebAssembly built, and this build made
/// again with it embedded from target/client, as every build of this checkout does from then on (the gateway serves what its build holds), as `{CLI} update` does for a release but at dev
/// speed: no release build, and cargo keeps what hasn't changed, so a run with nothing changed does nothing here. The
/// throwaway network's programs run the old build, so the command is run again as the new build, which starts it again (when
/// no client session is running).
fn web_client_built(checkout: &Path, rest: &[String]) -> Result<()> {
    let own = checkout.join("target/debug").join(env!("CARGO_BIN_NAME"));
    if std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) != own.canonicalize().ok() {
        return Ok(());
    }
    eprintln!("Building the web client's WebAssembly (the first time takes minutes; cargo keeps what hasn't changed)…");
    let built = crate::update::client(checkout, &checkout.join(one::config::CONFIG_FILE))?;
    let held = |name: &str, bytes: &[u8]| std::fs::read(built.join(name)).is_ok_and(|file| file == bytes);
    if !one::config::CLIENT.is_empty() && one::config::CLIENT.iter().all(|(name, bytes)| held(name, bytes)) {
        return Ok(());
    }
    rebuilt(checkout, rest, &format!("target/debug/{CLI} again with the web client"))
}

/// For `throwaway cloudflare node deploy` from this checkout's own debug build: the Cloudflare node's WebAssembly built (as a dev
/// build, not release) into target/cloudflare-node, as it is now (cargo keeps what hasn't changed), and this build made again
/// with it if it holds another, as `throwaway client web` does with the client's, so the two builds agree: every build of
/// this checkout holds both once they're there, and the throwaway network's relay (which names the Cloudflare node) starts for
/// any of them.
fn cloudflare_node_built(checkout: &Path, rest: &[String]) -> Result<()> {
    let own = checkout.join("target/debug").join(env!("CARGO_BIN_NAME"));
    if std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) != own.canonicalize().ok() {
        return Ok(());
    }
    eprintln!("Building the Cloudflare node's WebAssembly (the first time takes minutes; cargo keeps what hasn't changed)…");
    let built = crate::update::cloudflare_node(checkout, &checkout.join(one::config::CONFIG_FILE), "dev")?;
    let held = |name: &str, bytes: &[u8]| std::fs::read(built.join(name)).is_ok_and(|file| file == bytes);
    if !one::config::CLOUDFLARE_NODE.is_empty() && one::config::CLOUDFLARE_NODE.iter().all(|(name, bytes)| held(name, bytes)) {
        return Ok(());
    }
    rebuilt(checkout, rest, &format!("target/debug/{CLI} again with the Cloudflare node"))
}

/// Builds this checkout's debug build again, which takes the web client and Cloudflare node from target/ (build.rs, what
/// `cargo test` does too, so they build the same one) and watches the files there; the first time, it's watching none,
/// so touching build.rs has it look. The throwaway network's programs run the old build, so the command is run again as the new
/// build, which starts them again (when no client session is running).
fn rebuilt(checkout: &Path, rest: &[String], what: &str) -> Result<()> {
    let own = checkout.join("target/debug").join(env!("CARGO_BIN_NAME"));
    let again = format!("{}_THROWAWAY_REBUILT", CLI.to_uppercase());
    anyhow::ensure!(std::env::var_os(&again).is_none(), "{CLI} was built again and doesn't hold what it was built with: {what}");
    eprintln!("Building {what} (dev profile, not release)…");
    std::fs::File::options().write(true).open(checkout.join("build.rs"))?.set_modified(std::time::SystemTime::now())?;
    crate::update::run_quiet(std::process::Command::new("nice").arg(crate::update::cargo()).current_dir(checkout).env_remove("CLIENT").env_remove("CLOUDFLARE_NODE").arg("build"))?;
    use std::os::unix::process::CommandExt;
    Err(std::process::Command::new(&own).arg("throwaway").args(rest).env(&again, "1").exec().into())
}

/// Someone's name as `--as` gives it, as people write it: "sam" is Sam.
fn person_named(given: &str) -> Result<String> {
    let given = given.trim();
    anyhow::ensure!(!given.is_empty() && given.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-'), "--as takes a name, like --as sam");
    let mut chars = given.chars();
    Ok(chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default())
}

/// The web client's folder in the throwaway network's, for its first person or someone else.
fn web_folder(dir: &Path, person: Option<&str>) -> PathBuf {
    match person {
        None => dir.join("web"),
        Some(name) => dir.join(format!("web-{}", name.to_lowercase().replace(' ', "-"))),
    }
}

/// The web client, in a browser of the throwaway network's own on this machine, with no window: drive it over the DevTools
/// protocol. Its profile is kept with the throwaway network, so it stays signed in. As someone else, they're invited first,
/// by the throwaway network's first person, and join in it.
fn web(dir: &Path, settings: &one::cloudflare::Settings, person: Option<&str>) -> Result<()> {
    let (port, picture) = match person {
        Some(name) => joined(dir, settings, name)?,
        None => {
            let port = browser(dir, settings, None)?;
            let picture = dir.join("web.png");
            let code = web_script(dir, &["sign-in", &port.to_string(), &settings.address, &picture.display().to_string()])?;
            approve(dir, settings, port, &picture, &code)?;
            (port, picture)
        }
    };
    let who = person.map(|name| format!(" as {name}")).unwrap_or_default();
    println!(
        "The web client of this build is signed in to the throwaway network{who}, in a browser of its own on this machine.
  See it: {}
  Use it: its DevTools are at http://127.0.0.1:{port} (Playwright's connectOverCDP, or chrome-devtools-mcp --browserUrl)
  In any browser here: {}",
        picture.display(),
        settings.address
    );
    Ok(())
}

/// `name`'s web client, signed in: the first time, the throwaway network's machine invites them, as `devices invite` does on
/// an admin's machine (keeping the secret for their first device's approval), and they open the link in their own
/// browser, join, and sign in; then their browser, their first device, asks to join and the machine approves it. Pictures
/// of the invite and joining are kept beside theirs. → its DevTools port, and the picture of their first screen.
fn joined(dir: &Path, settings: &one::cloudflare::Settings, name: &str) -> Result<(u16, PathBuf)> {
    let folder = web_folder(dir, Some(name));
    let picture = folder.with_extension("png");
    let port = browser(dir, settings, Some(name))?;
    let mut args = vec!["sign-in".to_string(), port.to_string(), settings.address.clone(), picture.display().to_string()];
    let marker = folder.join("joined");
    if !marker.exists() {
        eprintln!("Inviting {name}…");
        let node = std::fs::read_to_string(dir.join("addr"))?;
        let invited = crate::request(throwaway_agent(dir).post(format!("http://{}/v1/invite-link", node.trim())).send_json(json!({ "name": name })))?;
        args.push(invited["link"].as_str().context("the throwaway network made no invite link")?.to_string());
    }
    let code = web_script(dir, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    std::fs::write(&marker, name)?;
    approve(dir, settings, port, &picture, &code).with_context(|| format!("{name}'s browser wasn't let in"))?;
    Ok((port, picture))
}

/// A web client that shows a code to be approved (`code`, else none) is approved by the throwaway network's machine, as its
/// person's device or as the device of the admin who invited them; then its window is pictured.
fn approve(dir: &Path, settings: &one::cloudflare::Settings, port: u16, picture: &Path, code: &str) -> Result<()> {
    if !code.is_empty() {
        approve_code(dir, code)?;
    }
    web_script(dir, &["picture", &port.to_string(), &settings.address, &picture.display().to_string()])?;
    Ok(())
}

/// Runs throwaway.js with Bun: → what it printed.
fn web_script(dir: &Path, args: &[&str]) -> Result<String> {
    let script = dir.join("web.js");
    std::fs::write(&script, include_str!("throwaway.js"))?;
    let out = std::process::Command::new(one::app::deploy::bun()?).arg(&script).args(args).output()?;
    anyhow::ensure!(out.status.success(), "the web client didn't sign in: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The throwaway network's browser for its first person or someone else, started if it isn't running: → its DevTools port.
fn browser(dir: &Path, settings: &one::cloudflare::Settings, person: Option<&str>) -> Result<u16> {
    let web = web_folder(dir, person);
    std::fs::create_dir_all(&web)?;
    let port = std::fs::read_to_string(web.join("port")).ok().and_then(|p| p.trim().parse::<u16>().ok());
    let answers = |port: u16| agent().get(format!("http://127.0.0.1:{port}/json/version")).call().is_ok_and(|r| r.status().is_success());
    let port = match port.filter(|p| answers(*p)) {
        Some(port) => port,
        None => {
            let log = format!("{}.log", web.file_name().unwrap().to_string_lossy());
            let mut chrome = one::app::deploy::chrome().context("the web client needs Chrome or Chromium on this machine: none was found (CHROME=<path> names one)")?;
            let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
            spawn(
                dir,
                chrome.args([
                    &format!("--remote-debugging-port={port}"),
                    &format!("--user-data-dir={}", web.join("profile").display()),
                    "--window-size=1280,860",
                    "--hide-scrollbars",
                    &settings.address,
                ]),
                &log,
                true,
            )?;
            std::fs::write(web.join("port"), port.to_string())?;
            let start = Instant::now();
            while !answers(port) {
                anyhow::ensure!(start.elapsed() < Duration::from_secs(30), "the throwaway network's browser didn't start: {}", dir.join(&log).display());
                std::thread::sleep(Duration::from_millis(250));
            }
            port
        }
    };
    Ok(port)
}

/// An Android or Windows client: on a throwaway copy of the person's lasting device, borrowed for the throwaway network,
/// which reaches the throwaway network's address at the same address; this checkout's build installed there, and signed in.
fn device_client(dir: &Path, checkout: &Path, settings: &one::cloudflare::Settings, kind: &str, person: Option<&str>) -> Result<()> {
    let mut borrowed = devices(dir);
    let what = borrowed_for(checkout, kind);
    let listed = listed_devices;
    // A copy borrowed for this checkout's throwaway network that it lost track of (the node restarted as it was lent) is
    // taken, not left beside a second one.
    let ours = |listed: &Value| listed.as_array().into_iter().flatten().find(|d| d["borrowed"]["for"] == what.as_str()).and_then(|d| d["name"].as_str()).map(String::from);
    let now = listed();
    let still = |name: &str| now.as_array().into_iter().flatten().any(|d| d["name"] == name);
    let device = match borrowed.get(kind).and_then(Value::as_str).filter(|d| still(d)).map(String::from).or_else(|| ours(&now)) {
        Some(device) => device,
        None => {
            eprintln!("Borrowing a copy of your {kind} device…");
            let got = match agent_devices("borrow", json!({ "kind": kind, "copy": true, "for": what })) {
                Ok(got) => got,
                Err(err) => match ours(&listed()) {
                    Some(device) => json!({ "device": device }),
                    None => return Err(err),
                },
            };
            let device = got["device"].as_str().context("Agent Devices lent no device")?.to_string();
            borrowed.insert(kind.into(), json!(device));
            std::fs::write(dir.join("devices.json"), serde_json::to_vec(&borrowed)?)?;
            device
        }
    };
    let port = settings.ports[1];
    agent_devices("reach", json!({ "device": device, "port": port }))?;
    // A native client reads who's who from the bucket itself, at the URLs the signer gives it: the throwaway bucket's port.
    if let Some(bucket) = std::fs::read_to_string(dir.join("bucket-addr")).ok().and_then(|a| a.trim().rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok())) {
        agent_devices("reach", json!({ "device": device, "port": bucket }))?;
    }
    // A copy is given back after 15 minutes without a call: a build may take longer.
    let building = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let keep = {
        let (building, device) = (building.clone(), device.clone());
        std::thread::spawn(move || {
            while building.load(std::sync::atomic::Ordering::Relaxed) {
                for _ in 0..300 {
                    std::thread::sleep(Duration::from_secs(1));
                    if !building.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                }
                let _ = agent_devices("reach", json!({ "device": device, "port": port }));
            }
        })
    };
    let built = match kind {
        "android" => android(dir, checkout, &device, settings, person),
        _ => windows(dir, checkout, &device, settings, person),
    };
    building.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = keep.join();
    built?;
    println!(
        "The {kind} client of this build is signed in to the throwaway network{}, on the device {device}, a throwaway copy borrowed for it.
  See it: {CLI} computer screen --device {device} (computer_screen), or {CLI} computer view --device {device} for a link
  Use it: computer_use with device {device} (computer_tools lists what it takes); {CLI} throwaway stop gives it back",
        person.map(|name| format!(" as {name}")).unwrap_or_default()
    );
    Ok(())
}

/// The native app (client/native) as this checkout builds it, on this machine in a display of its own (Xvfb), signed in
/// to the throwaway network as its first person or `person` as a person would: its Sign In pressed, the sign-in it would
/// open in a browser approved as them, and the code it then shows approved by the throwaway network's machine. What it
/// keeps (its folder, and its key in the Secret Service by the throwaway network's address) stays with the throwaway network, so a
/// new build is still signed in; it's started again each time.
fn linux(dir: &Path, checkout: &Path, settings: &one::cloudflare::Settings, person: Option<&str>) -> Result<()> {
    let address = settings.address.as_str();
    let name = person.map(|p| format!("linux-{}", p.to_lowercase().replace(' ', "-"))).unwrap_or_else(|| "linux".into());
    let home = dir.join(&name);
    std::fs::create_dir_all(&home)?;
    eprintln!("Building client/native (dev profile; the first time takes minutes)…");
    crate::update::run_quiet(std::process::Command::new("nice").arg(crate::update::cargo()).arg("build").current_dir(checkout.join("client/native")))?;
    // Its own display and control port, the same each time for this throwaway network and person.
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(home.to_string_lossy().as_bytes()).into();
    let display = format!(":{}", 200 + u16::from_le_bytes([hash[0], hash[1]]) % 600);
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    // What ran before for it goes: the app, and its display.
    for file in ["app.pid", "display.pid"] {
        if let Some(pid) = std::fs::read_to_string(home.join(file)).ok().and_then(|p| p.trim().parse::<i32>().ok()) {
            unsafe { libc::kill(-pid, libc::SIGTERM) };
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    let xvfb = spawn(dir, std::process::Command::new("Xvfb").args([display.as_str(), "-screen", "0", "1280x800x24", "-nolisten", "tcp"]), &format!("{name}/display.log"), true)?;
    std::fs::write(home.join("display.pid"), xvfb.to_string())?;
    std::thread::sleep(Duration::from_secs(1));
    let env = |key: &str| format!("{}_{key}", CLI.to_uppercase());
    let log = format!("{name}/app.log");
    let app = spawn(
        dir,
        // On a session bus of its own, with a Secret Service there (GNOME's keyring, unlocked with a password of no worth, in its
        // folder), as a desktop has.
        std::process::Command::new("dbus-run-session")
            .args(["--", "sh", "-c", "printf throwaway | gnome-keyring-daemon --unlock --components=secrets >/dev/null && exec \"$0\""])
            .arg(checkout.join("client/native/target/debug/one-native"))
            .env("DISPLAY", &display)
            .env_remove("WAYLAND_DISPLAY")
            // What it writes of itself (its launcher entry, open at login, what opens `<cli>://`) stays in its folder.
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
            .env(env("ADDRESS"), address)
            .env(env("SIGN_IN_PRINT"), "1")
            .env(env("CONTROL"), control.to_string()),
        &log,
        true,
    )?;
    std::fs::write(home.join("app.pid"), app.to_string())?;
    let eval = |js: &str| agent().post(format!("http://127.0.0.1:{control}/eval")).send(js).map(drop);
    let start = Instant::now();
    let (mut signed, mut approved) = (false, false);
    loop {
        anyhow::ensure!(start.elapsed() < Duration::from_secs(180), "the Linux app didn't sign in: {} says why", dir.join(&log).display());
        std::thread::sleep(Duration::from_secs(1));
        let said = std::fs::read_to_string(dir.join(&log)).unwrap_or_default();
        if said.lines().any(|l| l == "Let in.") {
            break;
        }
        match said.lines().rev().find_map(|l| l.strip_prefix("Sign in: ")) {
            Some(url) if !signed => {
                one::sign_in::approve_on_throwaway(address, url, person)?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the person.
            None => drop(eval("document.querySelector('[data-slot=welcome-action]')?.click()")),
        }
        if let Some(code) = said.lines().rev().find_map(|l| l.strip_prefix("Code: "))
            && !approved
        {
            approve_code(dir, code)?;
            approved = true;
        }
    }
    println!(
        "The Linux app of this build is signed in to the throwaway network{}, on this machine in the display {display}.
  See it: DISPLAY={display} import -window root shot.png
  Use it: DISPLAY={display} xdotool …, or run JavaScript in its page: curl -d '<js>' http://127.0.0.1:{control}/eval
  What it said: {}; {CLI} throwaway stop stops it",
        person.map(|name| format!(" as {name}")).unwrap_or_default(),
        dir.join(&log).display()
    );
    Ok(())
}

/// The Mac app (client/native) as this checkout builds it, on the Mac `MAC` names to ssh (`mac` if none), beside the
/// person's own app there: a throwaway's build (its own id, folder and Keychain items), in the Mac's logged-in session,
/// reaching the throwaway network at its address through ports ssh forwards there. Signed in as the throwaway network's
/// first person as linux() is: its Sign In pressed, the sign-in approved as them and opened there, its code approved.
/// What it keeps stays on the Mac, so a new build is still signed in; it's started again each time.
/// The Mac app, or with `iphone` the iPhone app on a simulator, of this checkout's build, built on the Mac `MAC` names
/// (ssh's `mac` by default), signed in to the throwaway network as its first person.
fn apple(dir: &Path, checkout: &Path, settings: &one::cloudflare::Settings, iphone: bool) -> Result<()> {
    let address = settings.address.as_str();
    let host = std::env::var("MAC").unwrap_or_else(|_| "mac".into());
    std::fs::write(dir.join(if iphone { "iphone" } else { "mac" }), &host)?;
    let ssh = |command: &str| -> Result<String> {
        let out = std::process::Command::new("ssh").args(["-o", "BatchMode=yes", &host, command]).output()?;
        anyhow::ensure!(out.status.success(), "on {host}: {command}: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    // This checkout as it is now, tracked and new files, in the Mac's cache, where its builds stay between runs.
    let there = format!("Library/Caches/{CLI}/throwaway-{}", checkout.file_name().unwrap_or_default().to_string_lossy());
    eprintln!("Copying this checkout to {host}:~/{there}…");
    let copied = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{{ git ls-files; git ls-files --others --exclude-standard; }} | sort -u | while read -r f; do [ -e \"$f\" ] && echo \"$f\"; done | tar -cz -T - | ssh -o BatchMode=yes {host} 'mkdir -p {there} && tar -xz -C {there}'"))
        .current_dir(checkout)
        .status()?;
    anyhow::ensure!(copied.success(), "couldn't copy this checkout to {host}");
    // The throwaway network's address and its bucket, at the same ports on the Mac, for as long as it runs.
    let port = |a: &str| a.trim().rsplit_once(':').and_then(|(_, p)| p.trim_end_matches('/').parse::<u16>().ok());
    let mut forward = vec!["-N".to_string(), "-o".into(), "BatchMode=yes".into(), "-o".into(), "ExitOnForwardFailure=yes".into()];
    for p in [port(address), std::fs::read_to_string(dir.join("bucket-addr")).ok().as_deref().and_then(port)].into_iter().flatten() {
        forward.extend(["-R".to_string(), format!("{p}:127.0.0.1:{p}")]);
    }
    forward.push(host.clone());
    if let Some(pid) = std::fs::read_to_string(dir.join("mac-forward.pid")).ok().and_then(|p| p.trim().parse::<i32>().ok()) {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
        std::thread::sleep(Duration::from_millis(500));
    }
    let tunnel = spawn(dir, std::process::Command::new("ssh").args(&forward), "mac-forward.log", true)?;
    std::fs::write(dir.join("mac-forward.pid"), tunnel.to_string())?;
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(format!("{}\n{host}", dir.display()).as_bytes()).into();
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    eprintln!("Building client/native on {host} (dev profile; the first time takes minutes)…");
    let script = if iphone { "iphone" } else { "mac" };
    let said = ssh(&format!("cd {there} && PATH=$HOME/.cargo/bin:/opt/homebrew/bin:$PATH client/native/{script}/throwaway.sh '{address}' {control}"))?;
    // The iPhone's says its simulator too.
    let (sim, log) = match said.lines().last().unwrap_or_default().rsplit_once(' ') {
        Some((sim, log)) if iphone => (sim.to_string(), log.to_string()),
        _ => (String::new(), said.lines().last().unwrap_or_default().to_string()),
    };
    let eval = |js: &str| ssh(&format!("curl -s -m 5 -d {} http://127.0.0.1:{control}/eval", shell_quote(js))).map(drop);
    let start = Instant::now();
    let (mut signed, mut approved) = (false, false);
    loop {
        anyhow::ensure!(start.elapsed() < Duration::from_secs(180), "the app didn't sign in: {host}:{log} says why");
        std::thread::sleep(Duration::from_secs(2));
        let said = ssh(&format!("cat '{log}' 2>/dev/null || true"))?;
        if said.lines().any(|l| l.trim() == "Let in.") {
            break;
        }
        match said.lines().rev().find_map(|l| l.trim().strip_prefix("Sign in: ")) {
            // Approved as the person, then back to the app at its loopback port, which only the Mac reaches.
            Some(url) if !signed => {
                let back = one::sign_in::approved_on_throwaway(address, url, None)?;
                ssh(&format!("curl -s -m 30 -o /dev/null {}", shell_quote(&back)))?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the person.
            None => drop(eval("document.querySelector('[data-slot=welcome-action]')?.click()")),
        }
        if let Some(code) = said.lines().rev().find_map(|l| l.trim().strip_prefix("Code: "))
            && !approved
        {
            approve_code(dir, code)?;
            approved = true;
        }
    }
    if iphone {
        println!(
            "The iPhone app of this build is signed in to the throwaway network, on a simulator of its own on {host} ({sim}).
  See it: ssh {host} xcrun simctl io {sim} screenshot /tmp/shot.png
  Use it: run JavaScript in its page there: ssh {host} curl -d '<js>' http://127.0.0.1:{control}/eval; open a link: ssh {host} xcrun simctl openurl {sim} '{CLI}://…'
  What it said: {host}:{log}; {CLI} throwaway stop shuts the simulator down and deletes it"
        );
        return Ok(());
    }
    println!(
        "The Mac app of this build is signed in to the throwaway network, on {host} beside the app there (its menu bar item, its window).
  See it: ssh {host} screencapture -x /tmp/shot.png
  Use it: run JavaScript in its page there: ssh {host} curl -d '<js>' http://127.0.0.1:{control}/eval (/eval/panel, the panel's)
  What it said: {host}:{log}; {CLI} throwaway stop stops it"
    );
    Ok(())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Runs a build script of the checkout's, at low priority, as other threads share this machine.
fn build(checkout: &Path, script: &str, envs: &[(&str, &str)]) -> Result<String> {
    eprintln!("Building {script}…");
    let out = std::process::Command::new("nice").arg(checkout.join(script)).envs(envs.iter().copied()).current_dir(checkout).stderr(std::process::Stdio::inherit()).output()?;
    anyhow::ensure!(out.status.success(), "{script} failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The Android app (client/native), as this checkout builds it (client/native/android/build.sh, for the processor the device
/// has), installed over the one there, then started with the throwaway network's address, printing its sign-in rather than
/// opening a Custom Tab, and with a control port, as linux() starts it (its settings go in as the intent's `env.` extras,
/// which a debug build reads): its Sign In pressed, the sign-in approved as the throwaway network's first person or `person`,
/// coming back to it by the app's own link, opened on the device, and its code approved by the throwaway network's machine.
/// Once signed in on a device as them (a file in the throwaway network's folder says so), a new build keeps that sign-in.
fn android(dir: &Path, checkout: &Path, device: &str, settings: &one::cloudflare::Settings, person: Option<&str>) -> Result<()> {
    let address = settings.address.as_str();
    let abi = match on_device(device, "getprop ro.product.cpu.abi")?.trim() {
        abi @ ("x86_64" | "arm64-v8a") => abi.to_string(),
        other => bail!("this checkout builds the Android app for x86_64 and arm64-v8a, not {other}"),
    };
    let apk = build(checkout, "client/native/android/build.sh", &[("ABIS", abi.as_str())])?;
    let package = format!("app.{CLI}.android");
    agent_devices("install", json!({ "device": device, "app": apk.lines().last().unwrap_or_default() }))?;
    // The launcher keeps drawing home-screen widgets from the APK it replaced ("Can't load widget") until it restarts.
    on_device(device, "am force-stop com.google.android.apps.nexuslauncher")?;
    // Play Store updates WebView on a fresh copy a few minutes in, which kills every app using it: not while a test runs.
    on_device(device, "pm disable-user --user 0 com.android.vending")?;
    let signed_in = dir.join(format!("signed-in-{device}"));
    let first = settings.throwaway_person.as_ref().map(|(_, name)| name.as_str()).unwrap_or_default();
    let who = person.unwrap_or(first);
    let here = format!("{address}\n{who}");
    let kept = std::fs::read_to_string(&signed_in).is_ok_and(|was| was == here);
    // A copy of the person's phone may have a build signed in elsewhere, or as someone else: what it kept goes.
    if !kept {
        on_device(device, &format!("pm clear {package}"))?;
    }
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(format!("{}\n{device}", dir.display()).as_bytes()).into();
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    let env = |key: &str| format!("{}_{key}", CLI.to_uppercase());
    on_device(device, "logcat -c")?;
    on_device(
        device,
        &format!(
            "am force-stop {package}; am start -W -n {package}/app.client.MainActivity --es env.{} '{address}' --es env.{} 1 --es env.{} {control}",
            env("ADDRESS"),
            env("SIGN_IN_PRINT"),
            env("CONTROL"),
        ),
    )?;
    // Its page's JavaScript, run through the control port, which only the device reaches.
    let click = "document.querySelector('[data-slot=welcome-action]')?.click()";
    let eval = |js: &str| on_device(device, &format!("(printf 'POST /eval HTTP/1.0\\r\\ncontent-length: {}\\r\\n\\r\\n'; printf '%s' \"{js}\") | nc -w 2 127.0.0.1 {control}", js.len()));
    let start = Instant::now();
    let (mut signed, mut approved) = (false, false);
    loop {
        anyhow::ensure!(start.elapsed() < Duration::from_secs(180), "the Android app didn't sign in: look at it with {CLI} computer screen --device {device}, and at what it said with {CLI} agent-devices run --device {device} \"logcat -d -s RustStdoutStderr\"");
        std::thread::sleep(Duration::from_secs(2));
        let said = on_device(device, "logcat -d -s RustStdoutStderr:I")?;
        let said: Vec<&str> = said.lines().filter_map(|l| l.split_once("RustStdoutStderr: ").map(|(_, s)| s.trim())).collect();
        if said.contains(&"Let in.") {
            break;
        }
        match said.iter().rev().find_map(|l| l.strip_prefix("Sign in: ")) {
            // Approved as the person, then back to the app by its own link, which opens it on the device.
            Some(url) if !signed => {
                let back = one::sign_in::approved_on_throwaway(address, url, person)?;
                on_device(device, &format!("am start -a android.intent.action.VIEW -n {package}/app.client.MainActivity -d '{back}'"))?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the person.
            None => drop(eval(click)),
        }
        if let Some(code) = said.iter().rev().find_map(|l| l.strip_prefix("Code: "))
            && !approved
        {
            // Someone else's device is approved by one of theirs (their browser, which joined first), not this machine's.
            match approve_code(dir, code) {
                Err(err) if person.is_some() && format!("{err:#}").contains("one of their own devices approves it") => {
                    println!(
                        "The Android app of this build is signed in to the throwaway network as {who}, on the device {device}, and waits for one of {who}'s own devices to let it in.
  Approve it as {who}: in their browser (`{CLI} throwaway client web --as {who}` makes it), open {address}/approve#{}, or {CLI} computer screen --device {device} for the code",
                        code.replace(' ', "")
                    );
                    return Ok(());
                }
                approved => approved?,
            }
            approved = true;
        }
    }
    std::fs::write(&signed_in, &here)?;
    println!(
        "  Run JavaScript in its page: {CLI} agent-devices run --device {device} \"(printf 'POST /eval HTTP/1.0\\r\\ncontent-length: <length>\\r\\n\\r\\n'; printf '%s' '<js>') | nc -w 2 127.0.0.1 {control}\"; what it said: {CLI} agent-devices run --device {device} \"logcat -d -s RustStdoutStderr\"; open a link: {CLI} agent-devices run --device {device} \"am start -a android.intent.action.VIEW -d '{CLI}://…' {package}\""
    );
    Ok(())
}

/// Whether text is the code a new device shows to be approved: "ABCD EFGH".
fn is_code(text: &str) -> bool {
    text.len() == 9 && text.char_indices().all(|(i, c)| if i == 4 { c == ' ' } else { c.is_ascii_uppercase() || ('2'..='7').contains(&c) })
}

/// A native client's code, approved by the throwaway network's machine, as its person's device: then it's let in.
fn approve_code(dir: &Path, code: &str) -> Result<()> {
    eprintln!("Approving the device that shows {code}…");
    let node = std::fs::read_to_string(dir.join("addr"))?;
    let approved = crate::request(throwaway_agent(dir).post(format!("http://{}/v1/devices/approve", node.trim())).send_json(json!({ "code": code.replace(' ', ""), "confirm": true })));
    // Let in already, by its sign-in, by that same machine: nothing waits.
    match approved {
        Err(err) if format!("{err:#}").contains("no device is waiting") => Ok(()),
        r => r.map(drop),
    }
}

/// The Windows app (client/native): cross-built here and packaged with its helper in the device
/// (client/native/windows/build.sh), installed there, then started by its program's path in the package (so it runs as
/// the package, as its badge and widgets need), at the throwaway network's address, printing its sign-in rather than
/// opening a browser, and with a control port, as linux() starts it: its Sign In pressed, the sign-in approved as the
/// throwaway network's first person or `person`, coming back to it on the device, and its code approved by the
/// throwaway network's machine. Once signed in on a device as them (a file in the throwaway network's folder says so), a
/// new build keeps that sign-in.
fn windows(dir: &Path, checkout: &Path, device: &str, settings: &one::cloudflare::Settings, person: Option<&str>) -> Result<()> {
    let address = settings.address.as_str();
    build(checkout, "client/native/windows/build.sh", &[("DEVICE", device), ("PROFILE", "dev"), ("NO_START", "1")])?;
    let file = NAME.chars().filter(char::is_ascii_alphanumeric).collect::<String>();
    let signed_in = dir.join(format!("signed-in-{device}"));
    let first = settings.throwaway_person.as_ref().map(|(_, name)| name.as_str()).unwrap_or_default();
    let who = person.unwrap_or(first);
    let here = format!("{address}\n{who}");
    let kept = std::fs::read_to_string(&signed_in).is_ok_and(|was| was == here);
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(format!("{}\n{device}", dir.display()).as_bytes()).into();
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    let env = |key: &str| format!("{}_{key}", CLI.to_uppercase());
    // A copy of the person's computer may have the app signed in elsewhere, or as someone else: what it kept goes (its
    // folder, the package's, and its sign-in in the Credential Manager), and it signs in again. Started on the device's
    // desktop, as a task signed in there (a window started from SSH never draws), as build.ps1 does; what it says, in a
    // log there.
    let script = format!(
        r#"$p = Get-AppxPackage -Name '{file}'
Get-Process -Name '{file}' -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500
$data = "$env:LOCALAPPDATA\Packages\$($p.PackageFamilyName)\LocalCache\Local\app.{CLI}.windows"
if (${fresh}) {{
  Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
  cmdkey /delete:'refresh.{CLI} sign-in' | Out-Null
}}
# The address, kept where the one a person types is, so what runs beside it (the widgets' helper) finds it too.
New-Item -ItemType Directory -Force $data | Out-Null
Set-Content "$data\address" '{address}' -NoNewline
$env:{print_var} = '1'
$env:{control_var} = '{control}'
Start-Process -FilePath "$($p.InstallLocation)\{file}.exe" -RedirectStandardOutput "$env:TEMP\one-throwaway-app.log" -RedirectStandardError "$env:TEMP\one-throwaway-app.err""#,
        fresh = if kept { "false" } else { "true" },
        print_var = env("SIGN_IN_PRINT"),
        control_var = env("CONTROL"),
    );
    let encoded = data_encoding::BASE64.encode(&script.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    on_device(
        device,
        &format!(
            r#"Remove-Item "$env:TEMP\one-throwaway-app.log" -ErrorAction SilentlyContinue
$a = New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -WindowStyle Hidden -EncodedCommand {encoded}'
$p = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive
Register-ScheduledTask -TaskName one-throwaway-app -Action $a -Principal $p -Force | Out-Null
Start-ScheduledTask -TaskName one-throwaway-app"#
        ),
    )?;
    let eval = |js: &str| on_device(device, &format!("try {{ Invoke-WebRequest -UseBasicParsing -Method Post -Body '{}' http://127.0.0.1:{control}/eval | Out-Null }} catch {{ }}", js.replace('\'', "''")));
    let start = Instant::now();
    let (mut signed, mut approved) = (false, false);
    loop {
        anyhow::ensure!(start.elapsed() < Duration::from_secs(180), "the Windows app didn't sign in: look at it with {CLI} computer screen --device {device}, and at what it said in %TEMP%\\one-throwaway-app.log there");
        std::thread::sleep(Duration::from_secs(2));
        let said = on_device(device, r#"$log = "$env:TEMP\one-throwaway-app.log"; if (Test-Path $log) { Get-Content $log }"#)?;
        if said.lines().any(|l| l.trim() == "Let in.") {
            break;
        }
        match said.lines().rev().find_map(|l| l.trim().strip_prefix("Sign in: ")) {
            // Approved as the person, then back to the app at its loopback port, which only the device reaches.
            Some(url) if !signed => {
                let back = one::sign_in::approved_on_throwaway(address, url, person)?;
                on_device(device, &format!("Invoke-WebRequest -UseBasicParsing '{}' | Out-Null", back.replace('\'', "''")))?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the person.
            None => drop(eval("document.querySelector('[data-slot=welcome-action]')?.click()")),
        }
        if let Some(code) = said.lines().rev().find_map(|l| l.trim().strip_prefix("Code: "))
            && !approved
        {
            approve_code(dir, code)?;
            approved = true;
        }
    }
    std::fs::write(&signed_in, &here)?;
    println!("  Run JavaScript in its page: {CLI} agent-devices run --device {device} \"Invoke-WebRequest -UseBasicParsing -Method Post -Body '<js>' http://127.0.0.1:{control}/eval\"; what it said is in %TEMP%\\one-throwaway-app.log there");
    Ok(())
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
