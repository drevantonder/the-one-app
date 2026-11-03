//! `try`: an install of its own, made as `create` makes every new one, by a node beside this machine's own with its own
//! home and ports, so it touches nothing of the person's (src/agents.rs leaves the harnesses alone for a node given a
//! home). There's one for each checkout (or folder) it's run in, so threads working in different ones each have their
//! own. Its kernel runs beside it and signs in its first person with no passkey (kernel::Settings::try_person), so a
//! client of this checkout's build, on a device borrowed for it (`try client`), signs in with nobody there; others join
//! as anyone does, by an invite link the first makes, and again with no passkey (`try client <kind> --as <name>`). It
//! keeps its data until `try stop`: left alone it stops, and `try` starts it again as it was.

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
    // Help, and nothing started: try's own, or a built-in command's. An app's operation's needs the try install's apps.
    let help = rest.iter().any(|a| a == "--help" || a == "-h");
    match rest.first().map(String::as_str) {
        Some("--help" | "-h" | "help") | Some("stop" | "client" | "sign-in") if help || rest[0] == "help" => {
            println!("{}", usage());
            return Ok(());
        }
        Some(cmd) if help && crate::commands().iter().any(|c| c == cmd) => return crate::run(rest.to_vec()),
        _ => {}
    }
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
    if rest.iter().map(String::as_str).eq(["serverless", "deploy"]) {
        return serverless(&dir);
    }
    if rest.first().map(String::as_str) == Some("client") {
        let settings = load_kernel(&dir)?;
        return client(&dir, &checkout, &settings, &rest[1..]);
    }
    // Before any thread starts: the CLI has none. Its home, so the CLI asks its node with its token, as nobody signed in,
    // and its kernel's settings are its own kernel's.
    unsafe { std::env::set_var(addr_var(), &addr) };
    unsafe { std::env::set_var(format!("{}_HOME", CLI.to_uppercase()), &dir) };
    if rest.first().map(String::as_str) == Some("sign-in") {
        // As its person, with no passkey: the try install's kernel asks for none.
        let address = get("/v1/state")?["install"]["settings"]["address"].as_str().context("the try install has no address")?.to_string();
        let started = one::sign_in::start(&address)?;
        one::sign_in::approve_on_try(&started)?;
        one::sign_in::finish(started)?;
        let me = get("/v1/state")?["user"]["name"].as_str().unwrap_or("you").to_string();
        println!("Signed in as {me}. {CLI} try and {CLI} try mcp act as them until {CLI} try sign-out.");
        return Ok(());
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

{}",
        dir.display(),
        kernel.display(),
        usage()
    );
    Ok(())
}

/// Runs the try install's serverless node beside its kernel, in its workerd, as `serverless deploy` puts one in
/// Cloudflare: the gateway sends `/v1/` to it, and its devices answer its endpoint.
fn serverless(dir: &Path) -> Result<()> {
    anyhow::ensure!(one::serverless::modules().is_ok(), "this build has no serverless node: build it with SERVERLESS set, as {CLI} update does");
    let mut settings = load_kernel(dir)?;
    if !settings.serverless {
        settings.serverless = true;
        with_home(dir, || settings.save())?;
        stop(dir, false)?;
        start(dir)?;
    }
    let addr = running(dir).context("the try install didn't start again")?;
    let key = settings.serverless_endpoint().context("the try install's kernel has no serverless key")?;
    crate::request(try_agent(dir).post(format!("http://{addr}/v1/settings")).send_json(json!({ "name": "serverless", "value": key })))?;
    println!("The try install's serverless node runs beside its kernel, its endpoint {}; {} sends /v1/ to it.", &key[..8], settings.address);
    Ok(())
}

/// What `try` takes.
fn usage() -> String {
    format!(
        "Usage:
  {CLI} try                  start this checkout's try install, if it isn't running, and say where it is
  {CLI} try <command>        any command, in it: {CLI} try status, {CLI} try apps, {CLI} try board post hello
  {CLI} try mcp              its MCP server, for an agent: claude mcp add {CLI}-try -- {CLI} try mcp
  {CLI} try sign-in          sign its CLI in as its person, with no browser
  {CLI} try serverless deploy    run its serverless node beside its kernel, as Cloudflare would
  {CLI} try client <kind>    a signed-in client of this checkout's build: web, android or windows
  {CLI} try client <kind> --as sam    the same, as someone else, invited by you with Add Someone's link
  {CLI} try stop             stop and delete it, and give back its devices; {CLI} try then starts a new one"
    )
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

/// A new try install, and its kernel.
fn start_new(dir: &Path) -> Result<String> {
    eprintln!("Starting a new try install for this checkout…");
    stop(dir, true)?;
    // Called what the person is called in their own install, if this machine is in one.
    let quick: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).middleware(one::api::as_this_machine).build().into();
    let mine = quick.get(format!("http://{}/v1/state", crate::addr())).call().ok().and_then(|mut r| r.body_mut().read_json::<Value>().ok());
    let me = mine.and_then(|s| s["user"]["name"].as_str().or(s["me"]["name"].as_str()).map(String::from));
    let me = me.or_else(|| std::env::var("USER").ok()).unwrap_or_else(|| "You".into());
    let free = std::net::TcpListener::bind(("127.0.0.1", one::config::PORT + 100)).is_ok().then(|| format!("127.0.0.1:{}", one::config::PORT + 100));
    std::fs::create_dir_all(dir)?;
    let bucket = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string();
    std::fs::write(dir.join("bucket-addr"), &bucket)?;
    let addr = spawn_node(dir, free.as_deref())?;
    let agent = || try_agent(dir);
    let on = |path: &str| format!("http://{addr}{path}");
    let created = crate::request(agent().post(on("/v1/create")).send_json(json!({ "name": "Try", "user_name": me })))?;
    let recovery = created["recovery"].as_str().context("the try install gave no recovery key")?;
    write_private(&dir.join("recovery-key"), &format!("{recovery}\n"))?;
    let state = crate::request(agent().get(on("/v1/state")).call())?;
    let person = (state["user"]["id"].as_str().context("the try install has no person")?.to_string(), me);
    let settings = one::kernel::Settings::for_try(&addr, &bucket, person)?;
    with_home(dir, || settings.save())?;
    // Read back as the kernel reads it, with its secrets.
    let settings = load_kernel(dir)?;
    spawn_kernel(dir, &settings)?;
    crate::request(agent().post(on("/v1/settings")).send_json(json!({ "name": "address", "value": &settings.address })))?;
    // Its data moves to the try bucket, which its node reaches only through the kernel's URL signer from then on.
    let moved = crate::request(agent().post(on("/v1/storage")).send_json(json!({ "endpoint": format!("http://{bucket}"), "bucket": "try" })))?;
    anyhow::ensure!(moved["kept"] == true, "the try install's node couldn't reach its bucket through the URL signer: {moved}");
    let started = std::time::Instant::now();
    while crate::request(agent().get(on("/v1/storage")).call())?["kind"] != "s3" {
        anyhow::ensure!(started.elapsed() < Duration::from_secs(30), "the try install's node didn't move to its bucket: node.log says why");
        std::thread::sleep(Duration::from_millis(200));
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
    wait_for(&try_agent(dir), &format!("http://{addr}/v1/state"), &dir.join("node.log"))?;
    Ok(addr)
}

/// A client asking the try install's node as one of this machine's programs: with the token in its home.
fn try_agent(dir: &Path) -> ureq::Agent {
    let dir = dir.to_path_buf();
    let with_token = move |mut req: ureq::http::Request<ureq::SendBody>, next: ureq::middleware::MiddlewareNext| {
        if let Some(token) = one::api::local_token(&dir).and_then(|t| t.parse().ok()) {
            req.headers_mut().insert(one::api::LOCAL_TOKEN_HEADER, token);
        }
        next.handle(req)
    };
    ureq::Agent::config_builder().http_status_as_error(false).middleware(with_token).build().into()
}

/// Starts the try install's kernel, and waits for its address to answer.
fn spawn_kernel(dir: &Path, settings: &one::kernel::Settings) -> Result<()> {
    for port in settings.ports {
        anyhow::ensure!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "the try install's kernel's port {port} is taken now: {CLI} try stop, then {CLI} try for a new one");
    }
    // Beside its node, which it asks for the install's devices, for its relay to take only them.
    let node = settings.node_url.as_deref().unwrap_or("").trim_start_matches("http://");
    spawn(dir, std::process::Command::new(std::env::current_exe()?).args(["kernel", "serve"]).env(addr_var(), node), "kernel.log")?;
    wait_for(&agent(), &format!("{}/.well-known/oauth-authorization-server", settings.address), &dir.join("kernel.log"))
}

fn wait_for(agent: &ureq::Agent, url: &str, log: &Path) -> Result<()> {
    let start = Instant::now();
    while !agent.get(url).call().is_ok_and(|r| r.status().is_success()) {
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

/// The try install's bucket, at `bucket-addr` in its folder: the folder its node made the install in, served S3's way
/// for the URLs its kernel's URL signer presigns (read, HEAD, list, create only if absent, replace while unchanged,
/// delete), signatures unchecked, as it's only this machine's and only a try install's.
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
    let place = one::bucket::Place::Folder(home.join("install").join("bucket"));
    let answer = move |method: Method, uri: Uri, headers: HeaderMap, body: axum::body::Bytes| {
        let place = place.clone();
        async move {
            if method == Method::GET && matches!(uri.path(), "/try" | "/try/") {
                let query: Vec<(String, String)> = uri.query().unwrap_or("").split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (decode(k), decode(v))).collect();
                let get = |k: &str| query.iter().find(|(q, _)| q == k).map(|(_, v)| v.clone());
                let (prefix, delimit) = (get("prefix").unwrap_or_default(), get("delimiter").is_some());
                let dir = home.join("install").join("bucket");
                return match tokio::task::spawn_blocking(move || list(&dir, &prefix, delimit)).await {
                    Ok(Ok(xml)) => (StatusCode::OK, HeaderMap::new(), xml.into_bytes()),
                    _ => (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new(), Vec::new()),
                };
            }
            let name = decode(uri.path().strip_prefix("/try/").unwrap_or(""));
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
    axum::serve(listener, axum::Router::new().fallback(answer)).await?;
    Ok(())
}

/// The names under `prefix` in the try bucket's folder `dir`, as S3's ListObjectsV2 answers, in one page: one level
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

/// The devices borrowed for the try install, by kind: `{ "android": "<name>" }`.
fn devices(dir: &Path) -> serde_json::Map<String, Value> {
    std::fs::read(dir.join("devices.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// One of this machine's install's Agent Devices' operations (not the try install's, which has none). When the node
/// restarts during it (an update), or its sidecar is still coming back, it's asked again once they answer.
fn agent_devices(op: &str, input: Value) -> Result<Value> {
    Ok(again(|| post("/v1/call", json!({ "app": "agent-devices", "op": op, "input": input, "client": "one try" })))?["ok"].clone())
}

/// What `ask` gives, asking again for up to 3 minutes while this machine's node or a sidecar of its restarts.
fn again(ask: impl Fn() -> Result<Value>) -> Result<Value> {
    let start = Instant::now();
    loop {
        match ask() {
            Err(err) if start.elapsed() < Duration::from_secs(180) && restarting(&format!("{err:#}")) => {
                eprintln!("{NAME} is restarting here; asking again once it's back…");
                std::thread::sleep(Duration::from_secs(5));
            }
            out => return out,
        }
    }
}

/// Whether an error is the node or a sidecar of it restarting, not the call failing.
fn restarting(err: &str) -> bool {
    ["isn't running here", "isn't running on this device", "no device runs agent-devices", "none of yours runs it"].iter().any(|s| err.contains(s))
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

/// `try client <kind> [--as <name>]`: a client of this checkout's build, signed in to the try install as its first
/// person, or as someone else they invited, and how to see and use it. Again, it's the same client, with this
/// checkout's build as it is now.
fn client(dir: &Path, checkout: &Path, settings: &one::kernel::Settings, rest: &[String]) -> Result<()> {
    let started = Instant::now();
    let person = match rest.iter().position(|a| a == "--as") {
        Some(at) => Some(person_named(rest.get(at + 1).context("--as who? --as sam")?)?),
        None => None,
    };
    // As its first person is as nobody else.
    let first = settings.try_person.as_ref().map(|(_, name)| name.to_lowercase());
    let person = person.filter(|name| Some(name.to_lowercase()) != first);
    match rest.first().map(String::as_str) {
        Some("web") => {
            // A browser reaches nodes over iroh, as the client's WebAssembly; a build without it would test the address.
            anyhow::ensure!(one::kernel::client_version().is_some(), "this build has no web client's WebAssembly: build it with CLIENT set, as {CLI} update does");
            web(dir, settings, person.as_deref())?
        }
        Some(kind @ ("android" | "windows")) => {
            if let Some(name) = &person {
                // They join where anyone does, in a browser: the try install's own here.
                joined(dir, settings, name)?;
            }
            device_client(dir, checkout, settings, kind, person.as_deref())?
        }
        _ => bail!("which client? {CLI} try client web, android or windows, with --as <name> for someone else"),
    }
    println!("Ready in {}s.", started.elapsed().as_secs());
    Ok(())
}

/// Someone's name as `--as` gives it, as people write it: "sam" is Sam.
fn person_named(given: &str) -> Result<String> {
    let given = given.trim();
    anyhow::ensure!(!given.is_empty() && given.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-'), "--as takes a name, like --as sam");
    let mut chars = given.chars();
    Ok(chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default())
}

/// The web client's folder in the try install's, for its first person or someone else.
fn web_folder(dir: &Path, person: Option<&str>) -> PathBuf {
    match person {
        None => dir.join("web"),
        Some(name) => dir.join(format!("web-{}", name.to_lowercase().replace(' ', "-"))),
    }
}

/// The web client, in a browser of the try install's own on this machine, with no window: drive it over the DevTools
/// protocol. Its profile is kept with the try install, so it stays signed in. As someone else, they're invited first,
/// by the try install's first person, and join in it.
fn web(dir: &Path, settings: &one::kernel::Settings, person: Option<&str>) -> Result<()> {
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
        "The web client of this build is signed in to the try install{who}, in a browser of its own on this machine.
  See it: {}
  Use it: its DevTools are at http://127.0.0.1:{port} (Playwright's connectOverCDP, or chrome-devtools-mcp --browserUrl)
  In any browser here: {}",
        picture.display(),
        settings.address
    );
    Ok(())
}

/// `name`'s web client, signed in: the first time, the try install's machine invites them, as `devices invite` does on
/// an admin's machine (keeping the secret for their first device's approval), and they open the link in their own
/// browser, join, and sign in; then their browser, their first device, asks to join and the machine approves it. Pictures
/// of the invite and joining are kept beside theirs. → its DevTools port, and the picture of their first screen.
fn joined(dir: &Path, settings: &one::kernel::Settings, name: &str) -> Result<(u16, PathBuf)> {
    let folder = web_folder(dir, Some(name));
    let picture = folder.with_extension("png");
    let port = browser(dir, settings, Some(name))?;
    let mut args = vec!["sign-in".to_string(), port.to_string(), settings.address.clone(), picture.display().to_string()];
    let marker = folder.join("joined");
    if !marker.exists() {
        eprintln!("Inviting {name}…");
        let node = std::fs::read_to_string(dir.join("addr"))?;
        let invited = crate::request(try_agent(dir).post(format!("http://{}/v1/invite-link", node.trim())).send_json(json!({ "name": name })))?;
        args.push(invited["link"].as_str().context("the try install made no invite link")?.to_string());
    }
    let code = web_script(dir, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    std::fs::write(&marker, name)?;
    approve(dir, settings, port, &picture, &code).with_context(|| format!("{name}'s browser wasn't let in"))?;
    Ok((port, picture))
}

/// A web client that shows a code to be approved (`code`, else none) is approved by the try install's machine, as its
/// person's device or as the device of the admin who invited them; then its window is pictured.
fn approve(dir: &Path, settings: &one::kernel::Settings, port: u16, picture: &Path, code: &str) -> Result<()> {
    if !code.is_empty() {
        let node = std::fs::read_to_string(dir.join("addr"))?;
        crate::request(try_agent(dir).post(format!("http://{}/v1/devices/approve", node.trim())).send_json(json!({ "code": code, "confirm": true })))?;
    }
    web_script(dir, &["picture", &port.to_string(), &settings.address, &picture.display().to_string()])?;
    Ok(())
}

/// Runs try_install.js with Bun: → what it printed.
fn web_script(dir: &Path, args: &[&str]) -> Result<String> {
    let script = dir.join("web.js");
    std::fs::write(&script, include_str!("try_install.js"))?;
    let out = std::process::Command::new(one::deploy::bun()?).arg(&script).args(args).output()?;
    anyhow::ensure!(out.status.success(), "the web client didn't sign in: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The try install's browser for its first person or someone else, started if it isn't running: → its DevTools port.
fn browser(dir: &Path, settings: &one::kernel::Settings, person: Option<&str>) -> Result<u16> {
    let web = web_folder(dir, person);
    std::fs::create_dir_all(&web)?;
    let port = std::fs::read_to_string(web.join("port")).ok().and_then(|p| p.trim().parse::<u16>().ok());
    let answers = |port: u16| agent().get(format!("http://127.0.0.1:{port}/json/version")).call().is_ok_and(|r| r.status().is_success());
    let port = match port.filter(|p| answers(*p)) {
        Some(port) => port,
        None => {
            let log = format!("{}.log", web.file_name().unwrap().to_string_lossy());
            let mut chrome = one::deploy::chrome().context("the web client needs Chrome or Chromium on this machine: none was found (CHROME=<path> names one)")?;
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
            )?;
            std::fs::write(web.join("port"), port.to_string())?;
            let start = Instant::now();
            while !answers(port) {
                anyhow::ensure!(start.elapsed() < Duration::from_secs(30), "the try install's browser didn't start: {}", dir.join(&log).display());
                std::thread::sleep(Duration::from_millis(250));
            }
            port
        }
    };
    Ok(port)
}

/// An Android or Windows client: on a throwaway copy of the person's lasting device, borrowed for the try install,
/// which reaches the try install's address at the same address; this checkout's build installed there, and signed in.
fn device_client(dir: &Path, checkout: &Path, settings: &one::kernel::Settings, kind: &str, person: Option<&str>) -> Result<()> {
    let mut borrowed = devices(dir);
    let what = format!("one try client {kind}, for {}", checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let listed = || again(|| post("/v1/call", json!({ "app": "computer", "op": "list", "input": {}, "client": "one try" }))).map(|l| l["ok"]["devices"].clone()).unwrap_or_default();
    // A copy borrowed for this checkout's try install that it lost track of (the node restarted as it was lent) is
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
    // A native client reads who's who from the bucket itself, at the URLs the signer gives it: the try bucket's port.
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
        "The {kind} client of this build is signed in to the try install{}, on the device {device}, a throwaway copy borrowed for it.
  See it: {CLI} computer screen --device {device} (computer_screen), or {CLI} computer view --device {device} for a link
  Use it: computer_use with device {device} (computer_tools lists what it takes); {CLI} try stop gives it back",
        person.map(|name| format!(" as {name}")).unwrap_or_default()
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
/// typing it and tapping Sign In, then the try install's sign-in going straight on, as the try install's first person
/// or `person`, whom the phone's browser signs in as first. Once signed in on a device as them (a file in the try
/// install's folder says so), a new build keeps that sign-in.
fn android(dir: &Path, checkout: &Path, device: &str, settings: &one::kernel::Settings, person: Option<&str>) -> Result<()> {
    let address = settings.address.as_str();
    let apk = build(checkout, "android/build.sh", &[])?;
    let apk = checkout.join("android").join(apk.lines().last().unwrap_or_default());
    let package = format!("app.{CLI}.android");
    agent_devices("install", json!({ "device": device, "app": apk.display().to_string() }))?;
    // The launcher keeps drawing home-screen widgets from the APK it replaced ("Can't load widget") until it restarts.
    on_device(device, "am force-stop com.google.android.apps.nexuslauncher")?;
    let signed_in = dir.join(format!("signed-in-{device}"));
    let first = settings.try_person.as_ref().map(|(_, name)| name.as_str()).unwrap_or_default();
    let who = person.unwrap_or(first);
    let here = format!("{address}\n{who}");
    let kept = std::fs::read_to_string(&signed_in).is_ok_and(|was| was == here);
    if !kept {
        // A copy of the person's phone may have a build signed in elsewhere, or as someone else.
        on_device(device, &format!("pm clear {package}"))?;
        // The app signs in through the phone's browser, which then signs in as whoever it last did here.
        let url = format!("{address}/sign-in?as={}", who.replace(' ', "%20"));
        on_device(device, &format!("am start -W -a android.intent.action.VIEW -d '{url}'"))?;
        let start = Instant::now();
        loop {
            anyhow::ensure!(start.elapsed() < Duration::from_secs(60), "the phone's browser didn't sign in as {who}: look at it with {CLI} computer screen --device {device}");
            std::thread::sleep(Duration::from_secs(2));
            let nodes = ui_nodes(&on_device(device, "uiautomator dump /sdcard/ui.xml >/dev/null 2>&1; cat /sdcard/ui.xml")?);
            if nodes.iter().any(|n| n.text.contains("is signed in here as")) {
                break;
            }
            if let Some(button) = ["Accept & continue", "No thanks", "Use without an account", "Got it"].iter().find_map(|t| nodes.iter().find(|n| n.text == *t)) {
                on_device(device, &format!("input tap {} {}", button.x, button.y))?;
            }
        }
    }
    on_device(device, &format!("am force-stop {package}; am start -W -n {package}/app.client.MainActivity"))?;
    let start = Instant::now();
    let (mut typed, mut pressed, mut approved) = (false, false, false);
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
        // Its code to be let in, which the try install's machine approves, as one of its person's devices.
        if ours && let Some(code) = nodes.iter().find(|n| is_code(&n.text)).filter(|_| !approved) {
            approve_code(dir, &code.text)?;
            approved = true;
            continue;
        }
        match find("Sign In").filter(|_| ours) {
            Some(button) => {
                on_device(device, &format!("input tap {} {}", button.x, button.y))?;
                pressed = true;
            }
            // Its home, once it signed in here, now or before.
            None if ours && (pressed || kept) && !nodes.iter().any(|n| n.class == "android.widget.EditText" || n.text.starts_with("Can’t Reach") || n.text.starts_with("Approve This")) => {
                std::fs::write(&signed_in, &here)?;
                return Ok(());
            }
            None => {}
        }
    }
}

/// Whether text is the code a new device shows to be approved: "ABCD EFGH".
fn is_code(text: &str) -> bool {
    text.len() == 9 && text.char_indices().all(|(i, c)| if i == 4 { c == ' ' } else { c.is_ascii_uppercase() || ('2'..='7').contains(&c) })
}

/// A native client's code, approved by the try install's machine, as its person's device: then it's let in.
fn approve_code(dir: &Path, code: &str) -> Result<()> {
    eprintln!("Approving the device that shows {code}…");
    let node = std::fs::read_to_string(dir.join("addr"))?;
    crate::request(try_agent(dir).post(format!("http://{}/v1/devices/approve", node.trim())).send_json(json!({ "code": code.replace(' ', ""), "confirm": true })))?;
    Ok(())
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
/// signed in at the try install's address as a person would: typing it and pressing Sign In, on the device's desktop,
/// as the try install's first person or `person`, whom Edge signs in as first. Once signed in on a device as them (a
/// file in the try install's folder says so), a new build keeps that sign-in.
fn windows(dir: &Path, checkout: &Path, device: &str, settings: &one::kernel::Settings, person: Option<&str>) -> Result<()> {
    let address = settings.address.as_str();
    build(checkout, "windows/build.sh", &[("DEVICE", device)])?;
    let file = NAME.chars().filter(char::is_ascii_alphanumeric).collect::<String>();
    let signed_in = dir.join(format!("signed-in-{device}"));
    let first = settings.try_person.as_ref().map(|(_, name)| name.as_str()).unwrap_or_default();
    let who = person.unwrap_or(first);
    let here = format!("{address}\n{who}");
    let kept = std::fs::read_to_string(&signed_in).is_ok_and(|was| was == here);
    // A copy of the person's computer may have the app signed in elsewhere, or as someone else: what it kept goes (the
    // app's own folder, or its package's, where it keeps it once packaged), and it signs in again. Edge, which it signs
    // in through, signs in as them first. Signed in, the sidebar has Settings. UI Automation reaches the desktop's
    // windows only from the desktop: as a task signed in there, as build.ps1 does.
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [Windows.Automation.AutomationElement]
$all = [Windows.Automation.TreeScope]::Descendants
$log = "$env:TEMP\one-try-sign-in.log"
function Find($in, $property, $value) {{ $in.FindFirst($all, (New-Object Windows.Automation.PropertyCondition($property, $value))) }}
function Window($process) {{
  $p = Get-Process -Name $process -ErrorAction SilentlyContinue | Where-Object MainWindowHandle -ne 0 | Select-Object -First 1
  if ($p) {{ $A::FromHandle($p.MainWindowHandle) }}
}}
function Texts($in) {{ if ($in) {{ ($in.FindAll($all, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object {{ $_.Current.Name }} | Where-Object {{ $_ }}) -join ' | ' }} }}
try {{
  if (${fresh}) {{
    Get-Process -Name '{file}' -ErrorAction SilentlyContinue | Stop-Process -Force
    Remove-Item "$env:LOCALAPPDATA\{NAME}\sign-in.json", "$env:LOCALAPPDATA\Packages\{file}_*\LocalCache\Local\{NAME}\sign-in.json" -ErrorAction SilentlyContinue
    Start-Process msedge '{address}/sign-in?as={as_}'
    $until = (Get-Date).AddSeconds(60)
    while ((Texts (Window msedge)) -notlike '*is signed in here as*') {{
      if ((Get-Date) -gt $until) {{ throw "Edge didn't sign in as {who}: $(Texts (Window msedge))" }}
      Start-Sleep 2
    }}
    Start-Process explorer.exe "shell:AppsFolder\$((Get-AppxPackage -Name '{file}').PackageFamilyName)!App"
  }}
  $until = (Get-Date).AddSeconds(120)
  $pressed = $false
  $said = $false
  while ($true) {{
    Start-Sleep 1
    $window = Window '{file}'
    if ((Get-Date) -gt $until) {{ throw "it didn't show its pages: $(Texts $window)" }}
    if (-not $window) {{ continue }}
    # Its code to be let in, said once for the try install's machine to approve.
    $code = ($window.FindAll($all, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object {{ $_.Current.Name }} | Where-Object {{ $_ -cmatch '^[A-Z2-7]{{4}} [A-Z2-7]{{4}}$' }} | Select-Object -First 1)
    if ($code -and -not $said) {{ "code $code" | Set-Content $log; $said = $true }}
    if (Find $window $A::NameProperty 'Settings') {{
      # Edge, left on the page it came back to the app from, closes, and the app is in front.
      Get-Process -Name msedge -ErrorAction SilentlyContinue | Where-Object MainWindowHandle -ne 0 | ForEach-Object {{ [void]$_.CloseMainWindow() }}
      Start-Sleep 2
      try {{ $window.SetFocus() }} catch {{ }}
      'signed in' | Set-Content $log
      exit
    }}
    # The welcome asks for the address first, with Continue, unless it has one; then Sign In, once.
    $field = Find $window $A::ControlTypeProperty ([Windows.Automation.ControlType]::Edit)
    $next = Find $window $A::NameProperty 'Continue'
    if ($field -and $next) {{
      $field.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('{address}')
      Start-Sleep -Milliseconds 500
      $next.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
      continue
    }}
    $button = Find $window $A::NameProperty 'Sign In'
    if ($button -and -not $pressed) {{
      $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
      $pressed = $true
    }}
  }}
}} catch {{ "failed: $_" | Set-Content $log }}"#,
        fresh = if kept { "false" } else { "true" },
        as_ = who.replace(' ', "%20"),
    );
    let encoded = data_encoding::BASE64.encode(&script.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    on_device(
        device,
        &format!(
            r#"Remove-Item "$env:TEMP\one-try-sign-in.log" -ErrorAction SilentlyContinue
$a = New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -WindowStyle Hidden -EncodedCommand {encoded}'
$p = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive
Register-ScheduledTask -TaskName one-try-sign-in -Action $a -Principal $p -Force | Out-Null
Start-ScheduledTask -TaskName one-try-sign-in"#
        ),
    )?;
    let start = Instant::now();
    let mut approved = false;
    loop {
        anyhow::ensure!(start.elapsed() < Duration::from_secs(240), "the Windows app didn't sign in: look at it with {CLI} computer screen --device {device}");
        std::thread::sleep(Duration::from_secs(3));
        match on_device(device, r#"$log = "$env:TEMP\one-try-sign-in.log"; if (Test-Path $log) { Get-Content $log }"#)?.trim() {
            "signed in" => break,
            "" => {}
            said if said.starts_with("code ") && !approved => {
                approve_code(dir, &said[5..])?;
                approved = true;
            }
            said if said.starts_with("code ") => {}
            said => bail!("the Windows app didn't sign in ({said}): look at it with {CLI} computer screen --device {device}"),
        }
    }
    std::fs::write(&signed_in, &here)?;
    Ok(())
}
