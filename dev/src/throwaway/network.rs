//! The throwaway network's life: starting its node and Workers, finding them running, stopping them.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use one::api::client::{Answer, Client, Via};
use one::config::CLI;
use one::cloudflare::Settings;
use one::mode::{Throwaway, programs};
use serde_json::json;

use crate::agent;

const BUILD: &str = "build";

/// Holds the throwaway network's folder locked until the returned file is dropped, or the process ends: starting it, stopping it
/// and what they write are one command's at a time, as starting a new one deletes what another command just wrote, and
/// two starting it at once start two. A command that finds it running again waits no longer than that. The folder itself is
/// locked, as stopping deletes what's in it but never it.
pub(crate) fn exclusive(dir: &Path) -> Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    let file = std::fs::File::open(dir).with_context(|| format!("locking {}", dir.display()))?;
    anyhow::ensure!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0, "locking {}: {}", dir.display(), std::io::Error::last_os_error());
    Ok(file)
}

/// Says which program the throwaway network in `dir` runs, and which folder its UI is read from, where they're given.
pub(crate) fn choose(dir: &Path, build: Option<&Path>, ui: Option<&Path>) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    if let Some(build) = build {
        std::fs::write(dir.join(BUILD), build.to_string_lossy().as_bytes())?;
    }
    if let Some(ui) = ui {
        Throwaway::set_ui(dir, ui)?;
    }
    Ok(())
}

/// Has the running throwaway network's Workers serve the web client's files from its UI folder as they are now, if it has
/// one: they're read as they start, and so a change since needs no restart (the Workers' own code does).
pub(crate) fn read_pages_again(dir: &Path) -> Result<()> {
    let Some(ui) = Throwaway::ui_in(dir) else { return Ok(()) };
    one::ui_folder::read_from(&ui);
    one::cloudflare::write_web(&Settings::load_in(dir)?)
}

/// The program the throwaway network in `dir` runs: the one it was given, else the one beside this.
pub(crate) fn program(dir: &Path) -> Result<PathBuf> {
    match std::fs::read_to_string(dir.join(BUILD)) {
        Ok(build) => Ok(build.trim().into()),
        Err(_) => crate::program(),
    }
}

/// The throwaway network's node's address, while its node runs and answers: an answer from the address alone could be
/// another checkout's throwaway network, which took the port while this one was stopped.
pub(crate) fn running(dir: &Path) -> Option<String> {
    let addr = std::fs::read_to_string(dir.join("addr")).ok()?.trim().to_string();
    let node: i32 = std::fs::read_to_string(dir.join("pid")).ok()?.trim().parse().ok()?;
    programs(dir, false).contains(&node).then_some(())?;
    node_at(dir, &addr).get("/v1/state").is_ok().then_some(addr)
}

/// Whether the throwaway network's node started before this build was made: it runs the build it started as, Workers and screens
/// included, so a new build (`cargo build`) is picked up by starting it again, which keeps its data.
pub(crate) fn older_build(dir: &Path) -> bool {
    let modified = |path: &Path| path.metadata().and_then(|m| m.modified()).ok();
    match (program(dir).ok().and_then(|exe| modified(&exe)), modified(&dir.join("pid"))) {
        (Some(built), Some(started)) => built > started,
        _ => false,
    }
}

/// Whether no client session (`throwaway client`, which holds `client.lock` shared while it builds and signs in) is running:
/// only then may a new build start the throwaway network again, as that drops what a client reaches it by.
pub(crate) fn no_client_running(dir: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(file) = std::fs::File::options().create(true).append(true).open(dir.join("client.lock")) else { return true };
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

/// Holds `client.lock` shared until the returned file is dropped, or the process ends: a client session is running.
pub(crate) fn client_session(dir: &Path) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::options().create(true).append(true).open(dir.join("client.lock")).ok()?;
    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH) } == 0).then_some(file)
}

/// Starts the throwaway network's node and Workers: as it was, if it stopped by itself, or a new one. → the node's address.
pub(crate) fn start(dir: &Path) -> Result<String> {
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
    spawn_cloudflare(dir, &Settings::load_in(dir)?, &addr)?;
    drop(held);
    spawn_node(dir, Some(&addr))
}

/// A TCP port held for the node, which isn't started yet: bound, not listening, so what asks for it is refused at once
/// where a listener would leave it waiting. → what holds it, until dropped, and its address.
pub(crate) fn reserve(addr: &str) -> Result<(std::os::fd::OwnedFd, String)> {
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
    let mut got: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let named = unsafe { libc::getsockname(fd, (&mut got as *mut libc::sockaddr_in).cast(), &mut len) };
    anyhow::ensure!(named == 0, "{}", std::io::Error::last_os_error());
    Ok((held, format!("{}:{}", addr.ip(), u16::from_be(got.sin_port))))
}

/// A new throwaway network, which is left only whole: one that fails to start (its Workers' code doesn't parse) is deleted,
/// so the next `throwaway` makes another where this one stopped, as its error says why.
pub(crate) fn start_new(dir: &Path) -> Result<String> {
    eprintln!("Starting a new throwaway network for this checkout…");
    let (build, ui) = (std::fs::read_to_string(dir.join(BUILD)).ok(), Throwaway::ui_in(dir));
    stop(dir, true)?;
    choose(dir, build.as_deref().map(Path::new), ui.as_deref())?;
    create(dir).inspect_err(|_| {
        let _ = stop(dir, true);
    })
}

fn create(dir: &Path) -> Result<String> {
    // Called what the user is called in their own network, if this machine is in one.
    let quick: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).middleware(one::api::as_this_machine).build().into();
    let mine = quick.get(format!("http://{}/v1/state", one::api::addr())).call().ok().and_then(|mut r| r.body_mut().read_json::<one::state::State>().ok());
    let me = mine.map(|s| s.looking().to_string());
    let me = me.or_else(|| std::env::var("USER").ok()).unwrap_or_else(|| "You".into());
    std::fs::create_dir_all(dir)?;
    let bucket = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string();
    std::fs::write(dir.join("bucket-addr"), &bucket)?;
    // A free port the system picks, as for everything else here: never a fixed one every checkout's network would
    // want first, which another's would hold when this one starts again.
    let addr = spawn_node(dir, None)?;
    let node = node_at(dir, &addr);
    let created = node.post("/v1/create", json!({ "name": "Throwaway", "user_name": me }))?;
    let recovery = created["recovery"].as_str().context("the throwaway network gave no recovery key")?;
    write_private(&dir.join("recovery-key"), &format!("{recovery}\n"))?;
    let user = (node.state()?.user.context("the throwaway network has no user")?.id, me);
    let settings = Settings::for_throwaway(dir, &bucket, user)?;
    settings.save()?;
    // Read back as `cloudflare deploy` reads it, with its secrets.
    let settings = Settings::load_in(dir)?;
    spawn_cloudflare(dir, &settings, &addr)?;
    node.post("/v1/settings", json!({ "name": "address", "value": &settings.address }))?;
    // Its sign-in signs its own ID tokens, as a provider would, for this client: the one its users sign in with.
    let clients = format!("{}/api/auth one-throwaway", settings.address);
    node.post("/v1/settings", json!({ "name": one::onboarding::sign_in_proof::CLIENTS, "value": clients }))?;
    // Its data moves to the throwaway bucket, which its node reaches only through its URL signer from then on.
    let moved = node.post("/v1/storage", json!({ "endpoint": format!("http://{bucket}"), "bucket": "throwaway" }))?;
    anyhow::ensure!(moved["kept"] == true, "the throwaway network's node couldn't reach its bucket through the URL signer: {moved}");
    let started = std::time::Instant::now();
    while node.get("/v1/storage")?["kind"] != "s3" {
        anyhow::ensure!(started.elapsed() < Duration::from_secs(30), "the throwaway network's node didn't move to its bucket: node.log says why");
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(addr)
}

/// Starts a program of the throwaway network's in a process group of its own, which stopping it stops (`pids`), logging to
/// `log` in its folder. A client's browser is `client`: starting the network again leaves it running.
pub(crate) fn spawn(dir: &Path, command: &mut std::process::Command, log: &str, client: bool) -> Result<u32> {
    use std::os::unix::process::CommandExt;
    let log_path = dir.join(log);
    let log = std::fs::File::create(&log_path).with_context(|| format!("making {}", log_path.display()))?;
    let child = command.env(one::config::env("HOME"), dir).stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log).process_group(0).spawn().with_context(|| format!("starting {}", command.get_program().to_string_lossy()))?;
    one::mode::record(dir, child.id(), client)?;
    Ok(child.id())
}

/// Starts the throwaway network's node, at `addr` if it's given and free, and gives its address once it answers.
pub(crate) fn spawn_node(dir: &Path, addr: Option<&str>) -> Result<String> {
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
    let mut command = std::process::Command::new(program(dir)?);
    std::os::unix::process::CommandExt::arg0(&mut command, one::oned::name());
    Throwaway::tell(&mut command, dir, &addr);
    let node_program = spawn(dir, &mut command, "node.log", false)?;
    for (name, text) in [("pid", node_program.to_string()), ("addr", addr.clone())] {
        std::fs::write(dir.join(name), text).with_context(|| format!("writing {}", dir.join(name).display()))?;
    }
    let node = node_at(dir, &addr);
    wait_for(|| node.get("/v1/state").is_ok(), node_program, &dir.join("node.log"))?;
    Ok(addr)
}

/// The throwaway network's node, asked at `addr` as one of this machine's programs: with the token in its home.
pub(crate) fn node_at(dir: &Path, addr: &str) -> Client {
    Client::new(Direct { dir: dir.to_path_buf(), addr: addr.to_string() }, false)
}

/// The throwaway network's node, at the address its folder says.
pub(crate) fn node_in(dir: &Path) -> Result<Client> {
    Ok(node_at(dir, std::fs::read_to_string(dir.join("addr"))?.trim()))
}

struct Direct {
    dir: PathBuf,
    addr: String,
}

impl Via for Direct {
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, wait: Option<Duration>) -> Result<Answer> {
        let mut req = ureq::http::Request::builder().method(method).uri(format!("http://{}{path}", self.addr));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        if let Some(token) = one::api::local_token(&self.dir) {
            req = req.header(one::protocol::header::LOCAL_TOKEN, token);
        }
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let resp = agent.run(agent.configure_request(req.body(body.unwrap_or_default())?).timeout_global(wait).build())?;
        let trace = resp.headers().get(one::protocol::header::TRACE).and_then(|h| h.to_str().ok()).map(str::to_string);
        Ok(Answer { status: resp.status().as_u16(), trace, body: Box::new(resp.into_body().into_reader()) })
    }
}

/// Starts the throwaway network's Workers, beside its node at `node`, and waits for its address to answer.
pub(crate) fn spawn_cloudflare(dir: &Path, settings: &Settings, node: &str) -> Result<()> {
    for port in settings.ports {
        anyhow::ensure!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "the throwaway network's Workers' port {port} was taken by another program while it was stopped: {CLI}-dev throwaway stop, then {CLI}-dev throwaway for a new one on free ports");
    }
    // Beside its node, which it asks for the network's devices, for its relay to take only them.
    let mut workers = std::process::Command::new(program(dir)?);
    Throwaway::tell(workers.args(["cloudflare", "serve"]), dir, node);
    let program = spawn(dir, &mut workers, "cloudflare.log", false)?;
    let url = format!("{}/.well-known/oauth-authorization-server", settings.address);
    wait_for(|| agent().get(&url).call().is_ok_and(|r| r.status().is_success()), program, &dir.join("cloudflare.log"))
}

/// Waits for `ready`, while `program` runs: a program that has stopped (its code didn't parse) fails at once, with the end of
/// its `log`, which says why.
pub(crate) fn wait_for(ready: impl Fn() -> bool, program: u32, log: &Path) -> Result<()> {
    let start = Instant::now();
    while !ready() {
        // This process started it, so it's waited for here: a program that stopped is otherwise still there, as a zombie.
        let stopped = unsafe { libc::waitpid(program as i32, std::ptr::null_mut(), libc::WNOHANG) } != 0;
        if stopped || start.elapsed() >= Duration::from_secs(120) {
            let text = std::fs::read_to_string(log).unwrap_or_default();
            let tail: Vec<&str> = text.lines().rev().take(12).collect();
            let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
            let name = log.file_name().unwrap_or_default().to_string_lossy();
            let why = if stopped { "a program of it stopped" } else { "a program of it didn't answer in time" };
            bail!("the throwaway network didn't start: {why}. The end of {name}:\n{tail}");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// Stops the throwaway network's programs, if they run, and with `delete`, deletes all of it but what it downloaded or built.
pub(crate) fn stop(dir: &Path, delete: bool) -> Result<()> {
    stop_some(dir, delete, true)
}

/// `stop`, and with `clients` false, leaving the clients' browsers running, as starting the network again does.
pub(crate) fn stop_some(dir: &Path, delete: bool, clients: bool) -> Result<()> {
    let pids = programs(dir, clients);
    for pid in &pids {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
    }
    // Gone is every process of a program's group, not only the one that leads it: a Worker's workerd keeps its ports for as
    // long as it lives, and what starts next asks for them. A killed process is gone a moment after, not when it's killed.
    let gone = |within: Duration| {
        let start = Instant::now();
        // Those of ours that stopped are zombies until waited for.
        while pids.iter().any(|pid| unsafe {
            libc::waitpid(*pid as i32, std::ptr::null_mut(), libc::WNOHANG);
            libc::kill(-pid, 0) == 0
        }) && start.elapsed() < within {
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    gone(Duration::from_secs(10));
    for pid in &pids {
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    gone(Duration::from_secs(5));
    // What still runs stays listed: its browsers.
    let kept: String = std::fs::read_to_string(dir.join("pids")).unwrap_or_default().lines().filter(|l| !clients && l.ends_with(" client")).map(|l| format!("{l}\n")).collect();
    if kept.is_empty() {
        let _ = std::fs::remove_file(dir.join("pids"));
    } else {
        std::fs::write(dir.join("pids"), kept)?;
    }
    let _ = std::fs::remove_file(dir.join("pid"));
    if clients {
        super::clients::apple::stop(dir, delete);
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

pub(crate) fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    std::io::Write::write_all(&mut file, text.as_bytes())?;
    Ok(())
}
