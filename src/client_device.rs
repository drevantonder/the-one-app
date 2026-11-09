//! This computer's CLI and MCP as a client device of its user, where no node runs: signed in (`sign-in`), it asks to
//! be let in with the code it shows, as the native apps do, and once a device of theirs approved it, every request that
//! would go to this machine's node goes to theirs over iroh instead (`instead`), through `one_client::Device`. Its key is
//! kept beside the sign-in, where only this user reads it.

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use one::config::{CLI, NAME, YOURS};
use one_client::Device;
use serde_json::json;

static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("one-client").enable_all().build().expect("a runtime"));

/// Runs what `work` makes on the client's runtime and waits for it: on a thread of its own when this one is already a
/// runtime's, which can't wait on another.
fn run<T: Send, F: Future<Output = T>>(work: impl FnOnce() -> F + Send) -> T {
    if tokio::runtime::Handle::try_current().is_err() {
        return RUNTIME.block_on(work());
    }
    std::thread::scope(|s| s.spawn(|| RUNTIME.block_on(work())).join().expect("the client's work finishes"))
}

/// Where this computer keeps its device: beside the sign-in, one for each node address it'd ask, as the sign-in is.
fn dir() -> PathBuf {
    let signed_in = crate::sign_in::path();
    let name = signed_in.file_stem().unwrap_or_default().to_string_lossy().replacen("sign-in", "device", 1);
    signed_in.with_file_name(name)
}

/// Whether this machine's node answers: then everything goes to it, as it always has. `<CLI>_NO_NODE` says to take
/// none as there, for a CLI tried as a device on a computer that has one (`one-dev throwaway client cli`).
pub fn node_here() -> bool {
    static HERE: OnceLock<bool> = OnceLock::new();
    *HERE.get_or_init(|| {
        if std::env::var_os(one::config::env("NO_NODE")).is_some() {
            return false;
        }
        let addrs = std::net::ToSocketAddrs::to_socket_addrs(&one::api::addr()).map(|a| a.collect::<Vec<_>>()).unwrap_or_default();
        addrs.iter().any(|a| std::net::TcpStream::connect_timeout(a, Duration::from_millis(500)).is_ok())
    })
}

static KEPT: Mutex<Option<Arc<Device>>> = Mutex::new(None);

/// This computer's device, if it was signed in as one, for the network it was signed in at.
fn kept() -> Option<Arc<Device>> {
    let mut kept = KEPT.lock().unwrap();
    if kept.is_none() {
        let address = crate::sign_in::kept()?.address;
        let dir = dir();
        let secret = std::fs::read_to_string(dir.join("secret")).ok().and_then(|s| one::keys::unhex(s.trim()).ok())?;
        (std::fs::read_to_string(dir.join("address")).ok()?.trim() == address).then_some(())?;
        *kept = Some(Arc::new(Device::new(secret, &address, &dir.to_string_lossy())));
    }
    kept.clone()
}

fn forget() {
    let _ = std::fs::remove_dir_all(dir());
    *KEPT.lock().unwrap() = None;
}

/// Whether what this CLI asks goes to its user's nodes over iroh: it's a device of theirs, and no node runs here.
pub fn active() -> bool {
    !node_here() && kept().is_some()
}

/// This computer's own name, as it asks to be let in, and its key.
pub fn me() -> Option<(String, String)> {
    active().then(|| (crate::sign_in::hostname(), kept().map(|d| d.key()).unwrap_or_default()))
}

pub fn not_here() -> String {
    format!("{NAME} isn't running on this computer, and it isn't one of your devices yet: {CLI} sign-in --address <{YOURS}'s address> makes it one, or install {NAME} here to run apps on it")
}

/// What a request only this machine's own node answers says, asked from a computer that's only a device.
fn needs_node(path: &str) -> String {
    format!("this computer is one of your devices, not one that runs {NAME}, which {path} needs: do it on a computer that runs it")
}

/// Makes this computer one of its user's devices, at the network at `address`, where it's signed in: at once if it
/// already is, else it asks, shows its code, and waits for a device of theirs to approve it.
pub fn join(address: &str, invite: Option<&str>) -> Result<()> {
    let address = address.trim_end_matches('/');
    // One kept for another network starts over, with a new key.
    if std::fs::read_to_string(dir().join("address")).ok().as_deref().map(str::trim) != Some(address) {
        forget();
    }
    let mut device = kept_or_new(address)?;
    let d = device.clone();
    if run(move || async move { d.connect().await })? {
        return Ok(());
    }
    // One taken out starts over too, as a removed key isn't let in again.
    let d = device.clone();
    if run(move || async move { d.removed().await }) {
        forget();
        device = kept_or_new(address)?;
    }
    let token = crate::sign_in::token()?.with_context(|| format!("this computer is signed out: {CLI} sign-in"))?;
    let d = device.clone();
    let name = crate::sign_in::hostname();
    let invited = invite.map(str::to_string);
    run(move || async move { d.ask(&token, &name, invited.as_deref()).await })?;
    let code = device.code();
    let link = format!("{address}/approve#{}", code.replace(' ', ""));
    if invite.is_some() {
        println!("\nWhoever invited you approves this computer: show them this code, which they type in Settings › Devices, Approve\n\n  {code}");
    } else {
        println!("\nNow approve this computer as one of your devices: on another of them, in Settings › Devices, Approve, type\n\n  {code}\n\nor open {link} there.");
    }
    if invite.is_none() && crate::sign_in::has_screen() && crate::sign_in::open(&link) {
        println!("It's open in your browser here, to approve it there if that browser is one of your devices.");
    }
    println!("\nThe owner of this network can see everything in it.");
    println!("\nWaiting for {} to approve it…", if invite.is_some() { "them" } else { "you" });
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let token = crate::sign_in::token()?.with_context(|| format!("this computer was signed out: {CLI} sign-in"))?;
        let d = device.clone();
        if run(move || async move { d.wait(&token).await })? {
            return Ok(());
        }
    }
}

/// Makes this computer one of its user's devices by the network's recovery key, from a file, when no device is left
/// to say yes: it's signed in, so auth says whose it is.
pub fn join_by_recovery(address: &str, file: &str) -> Result<()> {
    let address = address.trim_end_matches('/');
    let recovery = one::keys::unhex(&one::keys::recovery_key_in(file)?)?;
    let device = kept_or_new(address)?;
    let token = crate::sign_in::token()?.with_context(|| format!("this computer is signed out: {CLI} sign-in"))?;
    let (d, name) = (device.clone(), crate::sign_in::hostname());
    run(move || async move { d.ask(&token, &name, None).await })?;
    let d = device.clone();
    run(move || async move { d.admit_by_recovery(&recovery).await })?;
    let d = device.clone();
    anyhow::ensure!(run(move || async move { d.connect().await })?, "The recovery key let this device in, but it can't reach {YOURS} yet. Run {CLI} sign-in again in a moment.");
    Ok(())
}

/// The device this computer signs in at `address` for, its key ‖ seal key (hex), which its sign-in names: the one it
/// has for that network, or a new one.
pub fn signing_in_for(address: &str) -> Result<String> {
    let address = address.trim_end_matches('/');
    if std::fs::read_to_string(dir().join("address")).ok().as_deref().map(str::trim) != Some(address) {
        forget();
    }
    let device = kept_or_new(address)?;
    let d = device.clone();
    // One taken out starts over as its user signs in again, so this sign-in names the new key.
    if run(move || async move { d.removed().await }) {
        forget();
        return Ok(kept_or_new(address)?.join());
    }
    Ok(device.join())
}

/// This computer's device for the network at `address`, made with a new key if it has none: signed in or not yet, as its
/// sign-in names it before it's signed in.
fn kept_or_new(address: &str) -> Result<Arc<Device>> {
    if let Some(device) = kept() {
        return Ok(device);
    }
    let dir = dir();
    let secret = || std::fs::read_to_string(dir.join("secret")).ok().and_then(|s| one::keys::unhex(s.trim()).ok());
    let here = std::fs::read_to_string(dir.join("address")).is_ok_and(|a| a.trim() == address);
    if secret().is_none() || !here {
        forget();
        keep(&dir, address)?;
    }
    let secret = secret().context("this computer's device wasn't kept")?;
    Ok(Arc::new(Device::new(secret, address, &dir.to_string_lossy())))
}

/// A new key for this computer, readable only by its user, for the network at `address`.
fn keep(dir: &std::path::Path, address: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let new = dir.join("secret.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&new)?;
    file.write_all(one::keys::hex(&one_client::new_secret()).as_bytes())?;
    file.sync_all()?;
    std::fs::write(dir.join("address"), address)?;
    std::fs::rename(&new, dir.join("secret"))?;
    Ok(())
}

/// Signs this computer out as a device: it takes itself out of its user's devices first, as every client's sign-out
/// does, then forgets its key. Whether it was one.
pub fn sign_out() -> bool {
    let Some(device) = kept() else {
        forget();
        return false;
    };
    run(move || async move { device.sign_out().await });
    forget();
    true
}

/// How many of this computer's commands may be connected at once: as many as a node lets a device (`CLIENT_CONNS` in
/// src/node/net.rs). The rest wait their turn, rather than be refused.
const AT_ONCE: usize = 4;

/// The turn this process holds while it's connected, given back at its end.
static TURN: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// An exclusive lock on `name` in this device's folder: at once if `wait` is false, else once it's free.
fn lock(name: &str, wait: bool) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(dir().join(name)).ok()?;
    let how = if wait { libc::LOCK_EX } else { libc::LOCK_EX | libc::LOCK_NB };
    (unsafe { libc::flock(file.as_raw_fd(), how) } == 0).then_some(file)
}

/// Connects this process, once: with a turn of its own, and one at a time, since the relay takes each signed request
/// once, by the millisecond it was signed in, and two commands started together would sign in the same one.
fn connected(device: &Arc<Device>) -> Result<bool, one_client::Error> {
    static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if ONCE.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(true);
    }
    let mut turn = TURN.lock().unwrap();
    if turn.is_none() {
        *turn = (0..AT_ONCE).find_map(|i| lock(&format!("turn-{i}"), false)).or_else(|| lock(&format!("turn-{}", std::process::id() as usize % AT_ONCE), true));
    }
    drop(turn);
    let _one_at_a_time = lock("connecting", true);
    let d = device.clone();
    let connected = run(move || async move { d.connect().await })?;
    ONCE.store(connected, std::sync::atomic::Ordering::Relaxed);
    Ok(connected)
}

/// Closes this computer's connections to its user's nodes, if it opened any, and gives back its turn: at the end
/// of each command.
pub fn end() {
    let Some(device) = KEPT.lock().unwrap().take() else { return };
    run(move || async move { device.disconnect().await });
    TURN.lock().unwrap().take();
}

/// What a refused request says: the network doesn't list this computer, yet or any more.
fn not_let_in() -> String {
    let removed = kept().is_some_and(|d| run(move || async move { d.removed().await }));
    match removed {
        true => format!("this computer was taken out of your devices: {CLI} sign-in lets it in again"),
        false => format!("this computer hasn't been approved as one of your devices yet: {CLI} sign-in asks again"),
    }
}

/// The nodes the last request that asked for a trace tried, as a client device sent it.
static TRIES: Mutex<Option<serde_json::Value>> = Mutex::new(None);

pub fn take_tries() -> Option<serde_json::Value> {
    TRIES.lock().unwrap().take()
}

fn answer(status: u16, error: String) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let body = ureq::Body::builder().mime_type("application/json").data(json!({ "error": error }).to_string());
    Ok(ureq::http::Response::builder().status(status).header("content-type", "application/json").body(body)?)
}

/// A ureq middleware, the one place where what this CLI asks of its node goes: to this machine's node, as one of its
/// own programs (`api::as_this_machine`); or while no node runs here and this computer is one of its user's devices,
/// to their nodes over iroh instead, as this device, never with this machine's token or a sign-in. Its body is read
/// before it's sent (to disk past `IN_MEMORY`, as a node may ask for it again with the keys it needs), and streamed
/// from there; the answer's read as it comes.
pub fn route(req: ureq::http::Request<ureq::SendBody>, next: ureq::middleware::MiddlewareNext) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let device = kept().filter(|_| !node_here());
    let (true, Some(device)) = (req.uri().authority().is_some_and(|a| a.as_str() == one::api::addr()), device) else {
        return one::api::as_this_machine(req, next);
    };
    let path = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    if !one::api::clients_may(req.uri().path()) && !one_client::answered(req.uri().path()) {
        return answer(403, needs_node(req.uri().path()));
    }
    let (parts, body) = req.into_parts();
    let sent = Spooled::read(body.into_reader())?;
    // Never this machine's own token, nor a sign-in's: the device's key is who it asks as.
    let headers: Vec<(String, String)> = parts
        .headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "authorization" | "host") && name.as_str() != one::protocol::header::LOCAL_TOKEN)
        .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
        .collect();
    match connected(&device) {
        Ok(true) => {}
        Ok(false) => return answer(403, not_let_in()),
        Err(err) => return Err(ureq::Error::Io(std::io::Error::other(format!("couldn't reach your devices: {err}")))),
    }
    let method = parts.method.to_string();
    let body = sent.sent();
    let fetched = run(move || async move { device.fetch(&method, &path, headers, body).await });
    // A request that asked for a trace (`--trace`): the nodes this device tried, which a node's own trace doesn't say.
    *TRIES.lock().unwrap() = match &fetched {
        Ok(response) => response.trace.as_ref().map(|t| t["tries"].clone()),
        Err(err) => err.trace.as_ref().map(|t| t["tries"].clone()),
    };
    let response = match fetched {
        Ok(response) => response,
        Err(err) if err.not_let_in => return answer(403, not_let_in()),
        Err(err) => return Err(ureq::Error::Io(std::io::Error::other(format!("couldn't reach your devices: {err}")))),
    };
    let mut built = ureq::http::Response::builder().status(response.status);
    let mut body = ureq::Body::builder();
    for (name, value) in &response.headers {
        if name == "content-type" {
            let mut said = value.split(';');
            body = body.mime_type(said.next().unwrap_or_default().trim());
            if let Some(charset) = said.find_map(|p| p.trim().strip_prefix("charset=")) {
                body = body.charset(charset);
            }
        }
        built = built.header(name, value);
    }
    Ok(built.body(body.reader(Streamed { body: Mutex::new(Some(response.body)), part: Vec::new(), at: 0 }))?)
}

/// The most of a request's body kept in memory: a bigger one (a git push's pack, a file) goes to disk, and streams to
/// the node from there, as only files and apps' own requests may be bigger (`CLIENT_BODY` in src/node/net.rs).
const IN_MEMORY: usize = 4 << 20;

/// A request's body, read: in memory, or past `IN_MEMORY`, in a file only this user reads, deleted once it's sent.
enum Spooled {
    Bytes(Vec<u8>),
    File(PathBuf),
}

impl Spooled {
    fn read(mut body: impl Read) -> std::io::Result<Spooled> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut sent = Vec::new();
        (&mut body).take(IN_MEMORY as u64 + 1).read_to_end(&mut sent)?;
        if sent.len() <= IN_MEMORY {
            return Ok(Spooled::Bytes(sent));
        }
        let path = std::env::temp_dir().join(format!("{CLI}-sending-{}", one::keys::hex(&rand::random::<[u8; 32]>())));
        let spooled = Spooled::File(path.clone());
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        file.write_all(&sent)?;
        std::io::copy(&mut body, &mut file)?;
        Ok(spooled)
    }

    fn sent(&self) -> one_client::Sent {
        match self {
            Spooled::Bytes(bytes) => one_client::Sent::Bytes(bytes.clone()),
            Spooled::File(path) => one_client::Sent::File(path.clone()),
        }
    }
}

impl Drop for Spooled {
    fn drop(&mut self) {
        if let Spooled::File(path) = self {
            let _ = std::fs::remove_file(path);
        }
    }
}

struct Streamed {
    body: Mutex<Option<one_client::Body>>,
    part: Vec<u8>,
    at: usize,
}

impl Read for Streamed {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.at >= self.part.len() {
            let Some(mut body) = self.body.get_mut().unwrap().take() else { return Ok(0) };
            let (body, part) = run(move || async move {
                let part = body.chunk().await;
                (body, part)
            });
            match part.map_err(|e| std::io::Error::other(e.to_string()))? {
                Some(part) => {
                    (self.part, self.at) = (part, 0);
                    *self.body.get_mut().unwrap() = Some(body);
                }
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.part.len() - self.at);
        out[..n].copy_from_slice(&self.part[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// How this computer reaches its network, for the bare command and `status`: as a device, through the node that answered.
pub fn said(state: &one::state::State) -> Option<String> {
    let (name, key) = me()?;
    let through = Some(state.device.name.as_str()).filter(|n| !n.is_empty()).unwrap_or("your devices");
    let me = state.user.as_ref().map_or("you", |u| u.name.as_str());
    let network = state.network.as_ref().map(|n| n.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| YOURS.to_string());
    Some(format!("{name} ({}), one of {me}'s devices in {network}, asking {through}", &key[..8.min(key.len())]))
}
