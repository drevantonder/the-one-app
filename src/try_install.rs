//! `try`: an install of its own, made as `create` makes every new one, by a node beside this machine's own with its own
//! home and ports, so it touches nothing of the person's (src/agents.rs leaves the harnesses alone for a node given a
//! home). There's one for each checkout (or folder) it's run in, so threads working in different ones each have their
//! own. Its kernel runs beside it and signs in its one person with no passkey (kernel::Settings::try_person), so a
//! client of this checkout's build, on a device borrowed for it (`try client`), signs in with nobody there. It keeps
//! its data until `try stop`: left alone it stops, and `try` starts it again as it was.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use one::config::{CLI, NAME};
use serde_json::{Value, json};

use crate::{agent, get, post};

/// The try install for the checkout (or folder) this is run in, beside the downloads every try install shares: → its
/// folder, and the checkout.
fn folder() -> (PathBuf, PathBuf) {
    let base = PathBuf::from(format!("{}-try", one::home().display()));
    let here = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let top = std::process::Command::new("git").args(["rev-parse", "--show-toplevel"]).output().ok().filter(|o| o.status.success());
    let checkout = top.map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())).unwrap_or(here);
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(checkout.to_string_lossy().as_bytes()).into();
    let hash = one::entry::hex(&hash);
    let name = checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    (base.join(format!("{name}-{}", &hash[..6])), checkout)
}

fn addr_var() -> String {
    format!("{}_ADDR", CLI.to_uppercase())
}

/// The try install's node's address, while it answers.
fn running(dir: &Path) -> Option<String> {
    let addr = std::fs::read_to_string(dir.join("addr")).ok()?.trim().to_string();
    agent().get(format!("http://{addr}/v1/state")).call().is_ok().then_some(addr)
}

pub fn try_install(rest: &[String]) -> Result<()> {
    let (dir, checkout) = folder();
    match rest.first().map(String::as_str) {
        Some("stop") => {
            give_back(&dir);
            stop(&dir, true)?;
            println!("Stopped and deleted this checkout's try install, and gave back its devices. {CLI} try starts a new one.");
            return Ok(());
        }
        Some(cmd @ ("serve" | "update" | "release" | "publish" | "try")) => bail!("{CLI} {cmd} isn't for the try install"),
        Some("kernel") if rest.get(1).map(String::as_str) == Some("serve") => bail!("the try install runs its own kernel"),
        _ => {}
    }
    let addr = match running(&dir) {
        Some(addr) => addr,
        None => start(&dir)?,
    };
    let kernel = dir.join("kernel").join("settings.json");
    if rest.first().map(String::as_str) == Some("client") {
        let settings = load_kernel(&dir)?;
        return client(&dir, &checkout, &settings, &rest[1..]);
    }
    // Before any thread starts: the CLI has none.
    unsafe { std::env::set_var(addr_var(), &addr) };
    if rest.first().map(String::as_str) == Some("kernel") {
        // Its kernel's settings, not this machine's own kernel's.
        unsafe { std::env::set_var(format!("{}_HOME", CLI.to_uppercase()), &dir) };
    }
    if !rest.is_empty() {
        return crate::run(rest.to_vec());
    }
    let state = get("/v1/state")?;
    let me = state["user"]["name"].as_str().or(state["me"]["name"].as_str()).unwrap_or("you");
    let address = state["install"]["settings"]["address"].as_str().unwrap_or("");
    println!(
        "This checkout's try install is running on this machine: a new install, Try, with only what every new install
starts with, and {me} as its admin, who signs in at {address} with no passkey. Its folder is {}, with its
recovery key; its kernel's settings are in {}.

  {CLI} try <command>        any command, in it: {CLI} try status, {CLI} try apps, {CLI} try board post hello
  {CLI} try mcp              its MCP server, for an agent: claude mcp add {CLI}-try -- {CLI} try mcp
  {CLI} try client <kind>    a signed-in client of this checkout's build: web, android or windows
  {CLI} try stop             stop and delete it, and give back its devices; {CLI} try then starts a new one",
        dir.display(),
        kernel.display()
    );
    Ok(())
}

/// The try install's kernel's settings, with its secrets, read as the kernel's own tools read them.
fn load_kernel(dir: &Path) -> Result<one::kernel::Settings> {
    with_home(dir, one::kernel::Settings::load)
}

/// `f`, with this CLI's home the try install's.
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

/// Starts the try install's node and kernel: as it was, if it stopped by itself, or a new one. → the node's address.
fn start(dir: &Path) -> Result<String> {
    let kept = dir.join("kernel").join("settings.json").exists() && dir.join("addr").exists();
    if !kept {
        return start_new(dir);
    }
    eprintln!("Starting this checkout's try install again…");
    stop(dir, false)?;
    let was = std::fs::read_to_string(dir.join("addr"))?.trim().to_string();
    // Its own statement: a listener made in spawn_node's arguments would hold the port while the node starts.
    let free = std::net::TcpListener::bind(&was).is_ok();
    let addr = spawn_node(dir, free.then_some(was.as_str()))?;
    let mut settings = load_kernel(dir)?;
    if settings.node_url.as_deref() != Some(&format!("http://{addr}")) {
        settings.node_url = Some(format!("http://{addr}"));
        with_home(dir, || settings.save())?;
    }
    spawn_kernel(dir, &settings)?;
    Ok(addr)
}

/// A new try install, its kernel, and its person's keys there.
fn start_new(dir: &Path) -> Result<String> {
    eprintln!("Starting a new try install for this checkout…");
    stop(dir, true)?;
    // Called what the person is called in their own install, if this machine is in one.
    let quick: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).build().into();
    let mine = quick.get(format!("http://{}/v1/state", crate::addr())).call().ok().and_then(|mut r| r.body_mut().read_json::<Value>().ok());
    let me = mine.and_then(|s| s["user"]["name"].as_str().or(s["me"]["name"].as_str()).map(String::from));
    let me = me.or_else(|| std::env::var("USER").ok()).unwrap_or_else(|| "You".into());
    let free = std::net::TcpListener::bind(("127.0.0.1", one::config::PORT + 100)).is_ok().then(|| format!("127.0.0.1:{}", one::config::PORT + 100));
    std::fs::create_dir_all(dir)?;
    let bucket = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string();
    std::fs::write(dir.join("bucket-addr"), &bucket)?;
    let addr = spawn_node(dir, free.as_deref())?;
    let on = |path: &str| format!("http://{addr}{path}");
    let created = crate::request(agent().post(on("/v1/create")).send_json(json!({ "name": "Try", "user_name": me })))?;
    let recovery = created["recovery"].as_str().context("the try install gave no recovery key")?;
    write_private(&dir.join("recovery-key"), &format!("{recovery}\n"))?;
    let state = crate::request(agent().get(on("/v1/state")).call())?;
    let person = (state["user"]["id"].as_str().context("the try install has no person")?.to_string(), me);
    let settings = one::kernel::Settings::for_try(&addr, &bucket, recovery, person)?;
    with_home(dir, || settings.save())?;
    // Read back as the kernel reads it, with its secrets.
    let settings = load_kernel(dir)?;
    spawn_kernel(dir, &settings)?;
    for (name, value) in [("address", &settings.address), ("kernel_vault", &settings.vault_url), ("kernel_schedules", &settings.schedules_url)] {
        crate::request(agent().post(on("/v1/settings")).send_json(json!({ "name": name, "value": value })))?;
    }
    // The person's keys at the vault first, so the install's own are given to it before any client signs in.
    settings.try_person_keys()?;
    let keys = crate::request(agent().post(on("/v1/kernel/keys")).send_json(json!({})))?;
    for given in one::kernel::give_keys(&settings, keys.as_array().map(Vec::as_slice).unwrap_or_default()) {
        given?;
    }
    Ok(addr)
}

/// Starts a program of the try install's in a process group of its own, which stopping it stops (`pids`), logging to
/// `log` in its folder.
fn spawn(dir: &Path, command: &mut std::process::Command, log: &str) -> Result<u32> {
    use std::os::unix::process::CommandExt;
    let log = std::fs::File::create(dir.join(log))?;
    let child = command.env(format!("{}_HOME", CLI.to_uppercase()), dir).stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log).process_group(0).spawn()?;
    let mut pids = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
    pids += &format!("{} {}\n", child.id(), started(child.id() as i32).unwrap_or_default());
    std::fs::write(dir.join("pids"), pids)?;
    Ok(child.id())
}

/// Starts the try install's node, at `addr` if it's given and free, and gives its address once it answers.
fn spawn_node(dir: &Path, addr: Option<&str>) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    // What every try install downloads or builds, shared.
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
        std::process::Command::new(std::env::current_exe()?).args(["serve", "--try"]).env(addr_var(), &addr),
        "node.log",
    )?;
    std::fs::write(dir.join("pid"), pid.to_string())?;
    std::fs::write(dir.join("addr"), &addr)?;
    wait_for(&format!("http://{addr}/v1/state"), &dir.join("node.log"))?;
    Ok(addr)
}

/// Starts the try install's kernel, and waits for its address to answer.
fn spawn_kernel(dir: &Path, settings: &one::kernel::Settings) -> Result<()> {
    for port in settings.ports {
        anyhow::ensure!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "the try install's kernel's port {port} is taken now: {CLI} try stop, then {CLI} try for a new one");
    }
    spawn(dir, std::process::Command::new(std::env::current_exe()?).args(["kernel", "serve"]), "kernel.log")?;
    wait_for(&format!("{}/.well-known/oauth-authorization-server", settings.address), &dir.join("kernel.log"))
}

fn wait_for(url: &str, log: &Path) -> Result<()> {
    let start = Instant::now();
    while !agent().get(url).call().is_ok_and(|r| r.status().is_success()) {
        anyhow::ensure!(start.elapsed() < Duration::from_secs(120), "the try install didn't start: {}", log.display());
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

/// The try install's programs that still run, by the process groups they lead (`spawn`): only those that started when
/// they did, where this machine says, as another process may have taken a pid since.
fn programs(dir: &Path) -> Vec<i32> {
    let pids = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
    let ours = |pid: i32, at: &str| at.is_empty() || started(pid).as_deref() == Some(at);
    pids.lines()
        .filter_map(|line| {
            let (pid, at) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
            Some((pid.parse().ok()?, at.to_string()))
        })
        .filter(|(pid, at)| ours(*pid, at))
        .map(|(pid, _)| pid)
        .collect()
}

/// Stops the try install's programs, if they run, and with `delete`, deletes all of it but what it downloaded or built.
pub fn stop(dir: &Path, delete: bool) -> Result<()> {
    let pids = programs(dir);
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
    let _ = std::fs::remove_file(dir.join("pids"));
    let _ = std::fs::remove_file(dir.join("pid"));
    if delete {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if entry.file_name() != "cache" {
                let path = entry.path();
                if path.is_dir() { std::fs::remove_dir_all(&path)? } else { std::fs::remove_file(&path)? }
            }
        }
    }
    Ok(())
}

/// Stops what the try install in `home` started besides its node, which then stops itself: its kernel and browser.
pub fn stop_others(home: &Path) {
    let me = std::process::id() as i32;
    for pid in programs(home) {
        if pid != me {
            unsafe { libc::kill(-pid, libc::SIGTERM) };
        }
    }
    let _ = std::fs::remove_file(home.join("pids"));
}

/// The try install's bucket as its kernel's vault reaches it, at `bucket-addr` in its folder: its node's folder of data,
/// served S3's way for presigned URLs (read, create only if absent, replace while unchanged, delete), signatures
/// unchecked, as it's only this machine's and only a try install's.
pub async fn bucket(home: PathBuf) -> Result<()> {
    use axum::http::{HeaderMap, Method, StatusCode, Uri};
    let Ok(at) = std::fs::read_to_string(home.join("bucket-addr")) else { return Ok(()) };
    let listener = tokio::net::TcpListener::bind(at.trim()).await?;
    let place = one::bucket::Place::Folder(home.join("install").join("bucket"));
    let answer = move |method: Method, uri: Uri, headers: HeaderMap, body: axum::body::Bytes| {
        let place = place.clone();
        async move {
            let name = decode(uri.path().strip_prefix("/try/").unwrap_or(""));
            if name.is_empty() || name.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
                return (StatusCode::BAD_REQUEST, HeaderMap::new(), Vec::new());
            }
            let header = move |h: &str| headers.get(h).and_then(|v| v.to_str().ok()).map(|v| v.trim_matches('"').to_string());
            let done = |made: bool| if made { StatusCode::OK } else { StatusCode::PRECONDITION_FAILED };
            let answered = tokio::task::spawn_blocking(move || -> Result<(StatusCode, HeaderMap, Vec<u8>)> {
                Ok(match method {
                    Method::GET => match place.get_object(&name)? {
                        Some(object) => (StatusCode::OK, [("etag".parse().unwrap(), format!("\"{}\"", object.tag).parse()?)].into_iter().collect(), object.bytes),
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
    axum::serve(listener, axum::Router::new().fallback(answer)).await?;
    Ok(())
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

/// The devices borrowed for the try install, by kind: `{ "android": "<name>" }`.
fn devices(dir: &Path) -> serde_json::Map<String, Value> {
    std::fs::read(dir.join("devices.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// One of this machine's install's Agent Devices' operations (not the try install's, which has none).
fn agent_devices(op: &str, input: Value) -> Result<Value> {
    let out = post("/v1/call", json!({ "app": "agent-devices", "op": op, "input": input, "client": "one try" }))?;
    Ok(out["ok"].clone())
}

/// A command run on a device, and what it said.
fn on_device(device: &str, command: &str) -> Result<String> {
    let out = agent_devices("run", json!({ "device": device, "command": command }))?;
    Ok(out.as_str().map(str::to_string).unwrap_or_else(|| out.to_string()))
}

/// Gives back every device borrowed for the try install.
fn give_back(dir: &Path) {
    for (kind, device) in devices(dir) {
        if let Some(device) = device.as_str()
            && let Err(err) = agent_devices("give_back", json!({ "device": device }))
        {
            eprintln!("Couldn't give back the {kind} device {device}: {err:#}");
        }
    }
}

/// `try client <kind>`: a client of this checkout's build, signed in to the try install as its person, and how to see
/// and use it. Again, it's the same client, with this checkout's build as it is now.
fn client(dir: &Path, checkout: &Path, settings: &one::kernel::Settings, rest: &[String]) -> Result<()> {
    let started = Instant::now();
    match rest.first().map(String::as_str) {
        Some("web") => web(dir, settings)?,
        Some(kind @ ("android" | "windows")) => device_client(dir, checkout, settings, kind)?,
        _ => bail!("which client? {CLI} try client web, android or windows"),
    }
    println!("Ready in {}s.", started.elapsed().as_secs());
    Ok(())
}

/// The web client, in a browser of the try install's own on this machine, with no window: drive it over the DevTools
/// protocol. Its profile is kept with the try install, so it stays signed in.
fn web(dir: &Path, settings: &one::kernel::Settings) -> Result<()> {
    let web = dir.join("web");
    std::fs::create_dir_all(&web)?;
    let port = std::fs::read_to_string(web.join("port")).ok().and_then(|p| p.trim().parse::<u16>().ok());
    let answers = |port: u16| agent().get(format!("http://127.0.0.1:{port}/json/version")).call().is_ok_and(|r| r.status().is_success());
    let port = match port.filter(|p| answers(*p)) {
        Some(port) => port,
        None => {
            let chrome = chrome().context("the web client needs Chrome or Chromium on this machine: none was found (CHROME=<path> names one)")?;
            let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
            spawn(
                dir,
                std::process::Command::new(chrome).args([
                    "--headless=new",
                    &format!("--remote-debugging-port={port}"),
                    &format!("--user-data-dir={}", web.join("profile").display()),
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--window-size=1280,860",
                    "--hide-scrollbars",
                    &settings.address,
                ]),
                "web.log",
            )?;
            std::fs::write(web.join("port"), port.to_string())?;
            let start = Instant::now();
            while !answers(port) {
                anyhow::ensure!(start.elapsed() < Duration::from_secs(30), "the try install's browser didn't start: {}", dir.join("web.log").display());
                std::thread::sleep(Duration::from_millis(250));
            }
            port
        }
    };
    let script = web.join("sign-in.js");
    std::fs::write(&script, include_str!("try_install.js"))?;
    let picture = dir.join("web.png");
    let out = std::process::Command::new(one::deploy::bun()?).arg(&script).args([&port.to_string(), &settings.address, &picture.display().to_string()]).output()?;
    anyhow::ensure!(out.status.success(), "the web client didn't sign in: {}", String::from_utf8_lossy(&out.stderr).trim());
    println!(
        "The web client of this build is signed in to the try install, in a browser of its own on this machine.
  See it: {}
  Use it: its DevTools are at http://127.0.0.1:{port} (Playwright's connectOverCDP, or chrome-devtools-mcp --browserUrl)
  In any browser here: {}",
        picture.display(),
        settings.address
    );
    Ok(())
}

/// Chrome or Chromium on this machine: CHROME's, the usual ones, or Playwright's.
fn chrome() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CHROME") {
        return Some(path.into());
    }
    let named = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"].iter().find_map(|name| {
        std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
    });
    let mac = PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    let playwright = || -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("HOME")?).join(".cache/ms-playwright");
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("chromium-"))).collect();
        found.sort();
        found.into_iter().rev().flat_map(|p| [p.join("chrome-linux64/chrome"), p.join("chrome-linux/chrome")]).find(|p| p.is_file())
    };
    named.or_else(|| mac.is_file().then_some(mac)).or_else(playwright)
}

/// An Android or Windows client: on a throwaway copy of the person's lasting device, borrowed for the try install,
/// which reaches the try install's address at the same address; this checkout's build installed there, and signed in.
fn device_client(dir: &Path, checkout: &Path, settings: &one::kernel::Settings, kind: &str) -> Result<()> {
    let mut borrowed = devices(dir);
    let listed = post("/v1/call", json!({ "app": "computer", "op": "list", "input": {}, "client": "one try" })).map(|l| l["ok"].clone()).unwrap_or_default();
    let still = |name: &str| listed["devices"].as_array().into_iter().flatten().any(|d| d["name"] == name);
    let device = match borrowed.get(kind).and_then(Value::as_str).filter(|d| still(d)) {
        Some(device) => device.to_string(),
        None => {
            eprintln!("Borrowing a copy of your {kind} device…");
            let what = format!("one try client {kind}, for {}", checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            let got = agent_devices("borrow", json!({ "kind": kind, "copy": true, "for": what }))?;
            let device = got["device"].as_str().context("Agent Devices lent no device")?.to_string();
            borrowed.insert(kind.into(), json!(device));
            std::fs::write(dir.join("devices.json"), serde_json::to_vec(&borrowed)?)?;
            device
        }
    };
    let port = settings.ports[3];
    agent_devices("reach", json!({ "device": device, "port": port }))?;
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
        "android" => android(dir, checkout, &device, &settings.address),
        _ => windows(checkout, &device, &settings.address),
    };
    building.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = keep.join();
    built?;
    println!(
        "The {kind} client of this build is signed in to the try install, on the device {device}, a throwaway copy borrowed for it.
  See it: {CLI} computer screen --device {device} (computer_screen), or {CLI} computer view --device {device} for a link
  Use it: computer_use with device {device} (computer_tools lists what it takes); {CLI} try stop gives it back"
    );
    Ok(())
}

/// Runs a build script of the checkout's, at low priority, as other threads share this machine.
fn build(checkout: &Path, script: &str, envs: &[(&str, &str)]) -> Result<String> {
    eprintln!("Building {script}…");
    let out = std::process::Command::new("nice").arg(checkout.join(script)).envs(envs.iter().copied()).current_dir(checkout).stderr(std::process::Stdio::inherit()).output()?;
    anyhow::ensure!(out.status.success(), "{script} failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The Android app: built, installed over the one there, started, and signed in at `address` as a person would:
/// typing it and tapping Sign In, then the try install's sign-in going straight on. Once signed in on a device (a
/// file in the try install's folder says so), a new build keeps that sign-in.
fn android(dir: &Path, checkout: &Path, device: &str, address: &str) -> Result<()> {
    let apk = build(checkout, "android/build.sh", &[])?;
    let apk = checkout.join("android").join(apk.lines().last().unwrap_or_default());
    let package = format!("app.{CLI}.android");
    agent_devices("install", json!({ "device": device, "app": apk.display().to_string() }))?;
    let signed_in = dir.join(format!("signed-in-{device}"));
    if !signed_in.exists() {
        // A copy of the person's phone may have a build signed in elsewhere.
        on_device(device, &format!("pm clear {package}"))?;
    }
    on_device(device, &format!("am force-stop {package}; am start -W -n {package}/app.client.MainActivity"))?;
    let start = Instant::now();
    let (mut typed, mut pressed) = (false, false);
    loop {
        anyhow::ensure!(start.elapsed() < Duration::from_secs(120), "the Android app didn't sign in: look at it with {CLI} computer screen --device {device}");
        std::thread::sleep(Duration::from_secs(2));
        let ui = on_device(device, "uiautomator dump /sdcard/ui.xml >/dev/null 2>&1; cat /sdcard/ui.xml")?;
        let nodes = ui_nodes(&ui);
        let find = |text: &str| nodes.iter().find(|n| n.text == text);
        let ours = nodes.iter().any(|n| n.package == package);
        if ours && let Some(field) = nodes.iter().find(|n| n.class == "android.widget.EditText").filter(|_| !typed) {
            // The welcome keeps the address it was last signed in at: anything else there goes first.
            if field.text != address {
                on_device(device, &format!("input tap {} {}", field.x, field.y))?;
                on_device(device, "input keycombination KEYCODE_CTRL_LEFT KEYCODE_A; input keyevent KEYCODE_DEL")?;
                on_device(device, &format!("input text '{address}'"))?;
            }
            typed = true;
            continue;
        }
        // The try install's sign-in page, where Chrome waits for a tap before going on to an app.
        if let Some(button) = nodes.iter().find(|n| n.text.starts_with("Sign In as ")).or_else(|| ["Accept & continue", "No thanks", "Use without an account", "Got it"].iter().find_map(|t| find(t))) {
            on_device(device, &format!("input tap {} {}", button.x, button.y))?;
            continue;
        }
        match find("Sign In").filter(|_| ours) {
            Some(button) => {
                on_device(device, &format!("input tap {} {}", button.x, button.y))?;
                pressed = true;
            }
            // Its home, once it signed in here, now or before.
            None if ours && (pressed || signed_in.exists()) && !nodes.iter().any(|n| n.class == "android.widget.EditText" || n.text.starts_with("Can’t Reach")) => {
                std::fs::write(&signed_in, address)?;
                return Ok(());
            }
            None => {}
        }
    }
}

/// A view on an Android screen, as uiautomator dumps it: its text, class and package, and its middle.
struct UiNode {
    text: String,
    class: String,
    package: String,
    x: i64,
    y: i64,
}

fn ui_nodes(xml: &str) -> Vec<UiNode> {
    let attr = |node: &str, name: &str| -> String {
        let key = format!(" {name}=\"");
        node.find(&key).map(|i| &node[i + key.len()..]).and_then(|s| s.split('"').next()).unwrap_or("").replace("&amp;", "&").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
    };
    xml.split("<node").skip(1).filter_map(|node| {
        let bounds: Vec<i64> = attr(node, "bounds").split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|n| n.parse().ok()).collect();
        let [x1, y1, x2, y2] = bounds[..] else { return None };
        Some(UiNode { text: attr(node, "text"), class: attr(node, "class"), package: attr(node, "package"), x: (x1 + x2) / 2, y: (y1 + y2) / 2 })
    }).collect()
}

/// The Windows app: built in the device itself (WinUI 3 builds only on Windows), which installs and starts it, then
/// signed in at `address` as a person would: typing it and pressing Sign In, on the device's desktop.
fn windows(checkout: &Path, device: &str, address: &str) -> Result<()> {
    build(checkout, "windows/build.sh", &[("DEVICE", device)])?;
    let name = NAME.chars().filter(char::is_ascii_alphanumeric).collect::<String>();
    // UI Automation reaches the desktop's windows only from the desktop: as a task signed in there, as build.ps1 does.
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [Windows.Automation.AutomationElement]
function Find($in, $property, $value) {{ $in.FindFirst([Windows.Automation.TreeScope]::Descendants, (New-Object Windows.Automation.PropertyCondition($property, $value))) }}
$log = "$env:TEMP\one-try-sign-in.log"
try {{
  $until = (Get-Date).AddSeconds(60)
  while ((Get-Date) -lt $until) {{
    $p = Get-Process -Name '{name}' -ErrorAction SilentlyContinue | Where-Object MainWindowHandle -ne 0 | Select-Object -First 1
    if ($p) {{
      $window = $A::FromHandle($p.MainWindowHandle)
      $button = Find $window $A::NameProperty 'Sign In'
      if (-not $button) {{ 'signed in' | Set-Content $log; exit }}
      $field = Find $window $A::ControlTypeProperty ([Windows.Automation.ControlType]::Edit)
      if ($field) {{ $field.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('{address}') }}
      Start-Sleep -Milliseconds 500
      $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
      'pressed' | Set-Content $log
      exit
    }}
    Start-Sleep -Seconds 1
  }}
  'no window' | Set-Content $log
}} catch {{ "failed: $_" | Set-Content $log }}"#
    );
    let encoded = data_encoding::BASE64.encode(&script.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    on_device(
        device,
        &format!(
            r#"Remove-Item "$env:TEMP\one-try-sign-in.log" -ErrorAction SilentlyContinue
$a = New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -WindowStyle Hidden -EncodedCommand {encoded}'
$p = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive
Register-ScheduledTask -TaskName one-try-sign-in -Action $a -Principal $p -Force | Out-Null
Start-ScheduledTask -TaskName one-try-sign-in
$until = (Get-Date).AddSeconds(90)
while (-not (Test-Path "$env:TEMP\one-try-sign-in.log") -and (Get-Date) -lt $until) {{ Start-Sleep 1 }}
Get-Content "$env:TEMP\one-try-sign-in.log" -ErrorAction SilentlyContinue"#
        ),
    )
    .and_then(|said| match said.trim() {
        "signed in" | "pressed" => Ok(()),
        said => bail!("couldn't press Sign In in the Windows app ({said}): look at it with {CLI} computer screen --device {device}"),
    })?;
    // The browser goes on by itself, back to the app at its own port.
    std::thread::sleep(Duration::from_secs(15));
    Ok(())
}
