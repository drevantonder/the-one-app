//! A service's sandbox: what its program may reach, as its app declares (`service: { network, user }` in its
//! app.js, `Declared`), and how this machine holds it to that (`How`). The service's endpoint (`oned <name>`,
//! src/services/endpoint.rs) holds its key and starts its program; the program never sees the key.
//!
//! - **Linux**, a service not run as the user: a unit of the template `<cli>-service@.service`, which `<cli> services
//!   sandbox` writes once as root (install.sh runs it) with a polkit rule letting the node's user start and stop
//!   only those. systemd gives it its own OS user (`DynamicUser=`), its own folder (`StateDirectory=`), no home and a
//!   read-only system. Its program, our launcher (`oned --sandboxed=<dir>`, `launch`) and a socket to its endpoint
//!   are put in its folder here (`dir`), which the unit sees read-only at `/run/<cli>`.
//! - **Linux**, one run as the user: the same launcher, started by its endpoint as the user.
//! - **Mac**: its program under a Seatbelt profile (`sandbox-exec`) written from its declaration, as the user: only
//!   its own folder, unless it runs as the user, and the network only through its endpoint.
//!
//! On Linux the launcher gives the program a network of its own with nothing in it (an unprivileged user and network
//! namespace), and forwards a port there to its endpoint's socket, where the program finds the services' API and a
//! proxy (`connect`) that reaches only the hosts its app declares (`HTTPS_PROXY`). A declared port on this machine
//! (`127.0.0.1:11434`) is forwarded at the same port. A service declaring `"*"` reaches any host; one also run as the
//! user runs as before, outside any sandbox.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::config::{CLI, NAME};

/// What an app declares its service may do: the hosts it may reach (`"*"`: any; `"*.example.com"`: that domain's;
/// `"host:port"`: only that port), and whether it runs as the node's user, with their files.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Declared {
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub user: bool,
}

impl Declared {
    pub fn any_host(&self) -> bool {
        self.network.iter().any(|h| h == "*")
    }

    pub fn allows(&self, host: &str, port: u16) -> bool {
        let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        self.network.iter().any(|d| {
            let d = d.to_ascii_lowercase();
            let (name, at) = match d.rsplit_once(':').filter(|(_, p)| p.parse::<u16>().is_ok()) {
                Some((name, p)) => (name.to_string(), p.parse::<u16>().ok()),
                None => (d.clone(), None),
            };
            let named = name == "*" || name == host || name.strip_prefix("*.").is_some_and(|s| host.ends_with(&format!(".{s}")));
            named && at.is_none_or(|p| p == port)
        })
    }

    /// The ports on this machine it declares (`127.0.0.1:11434`), which the launcher forwards at the same port.
    pub fn local_ports(&self) -> Vec<u16> {
        self.network.iter().filter_map(|d| d.strip_prefix("127.0.0.1:").or_else(|| d.strip_prefix("localhost:"))?.parse().ok()).collect()
    }
}

/// How a service's program runs on this machine.
#[derive(Clone, Debug, PartialEq)]
pub enum How {
    /// As the user, as a plain process: one declared to run as them with any host; or, saying why, one this machine
    /// can't sandbox.
    Plain(Option<String>),
    /// Linux: a unit of its own, as its own OS user.
    Unit,
    /// Linux: the launcher, as the user.
    User,
    /// Mac: under a Seatbelt profile.
    Seatbelt,
}

/// How the service `declared` runs here, on the device `device`; none declared is one whose app didn't say.
pub fn how(declared: Option<&Declared>, device: &str) -> How {
    let Some(d) = declared else {
        return How::Plain(Some("Its app doesn’t say what it may reach.".into()));
    };
    if d.user && d.any_host() {
        return How::Plain(None);
    }
    if cfg!(target_os = "macos") {
        return How::Seatbelt;
    }
    if d.user {
        return How::User;
    }
    match template().exists() {
        true => How::Unit,
        false => How::Plain(Some(format!("Install {NAME} again on {device} to give it one."))),
    }
}

/// Why it runs without its sandbox, when it should have one and this machine can't give it: what to do about it.
pub fn unsandboxed(declared: Option<&Declared>, device: &str) -> Option<String> {
    match how(declared, device) {
        How::Plain(why) => why,
        _ => None,
    }
}

pub fn template() -> PathBuf {
    PathBuf::from(format!("/etc/systemd/system/{CLI}-service@.service"))
}

/// The unit's instance for the service `name`: a name that's also its OS user's, unique to its folder, so a
/// throwaway network's never shares one with the real one's.
pub fn instance(name: &str) -> String {
    let hash = blake3::hash(crate::services::endpoint::folder(name).to_string_lossy().as_bytes()).to_hex();
    let long = format!("{CLI}-{name}-{}", &hash[..8]);
    if long.len() <= 31 { long } else { format!("{CLI}-{}", &hash[..16]) }
}

pub fn unit(name: &str) -> String {
    format!("{CLI}-service@{}.service", instance(name))
}

/// Where its sandbox's folders are: in the user's home, but not the node's, which a throwaway network has apart.
fn root() -> PathBuf {
    crate::user_home().join(".local/share").join(format!("{CLI}-services"))
}

/// The folder its program, the launcher and the socket to its endpoint are put in for it.
pub fn dir(name: &str) -> PathBuf {
    root().join(instance(name))
}

/// The script, run as root, that lets this machine's user start services in their sandbox: the unit template and
/// the polkit rule, and polkit itself if it's missing. `<cli> services sandbox | sudo sh`; install.sh runs it.
pub fn setup_script() -> Result<String> {
    let user = std::env::var("USER").ok().filter(|u| !u.is_empty() && u != "root").context("run it as the node's user, not root: <cli> services sandbox | sudo sh")?;
    ensure!(user.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)), "{user} isn't a user name this can write");
    let home = crate::user_home();
    let home = home.to_str().filter(|h| !h.contains(['\n', '"', '\'', '\\', '$', '`', ' '])).context("the home folder's path has characters a unit can't take")?;
    let unit = format!(
        "[Unit]
Description=A service of {NAME}, in its sandbox (%i)

[Service]
Type=exec
DynamicUser=yes
User=%i
StateDirectory={CLI}-services/%i
BindReadOnlyPaths={home}/.local/share/{CLI}-services/%i:/run/{CLI}
ExecStart=/run/{CLI}/{CLI}d --sandboxed=/run/{CLI}
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
ProtectProc=invisible
RestrictSUIDSGID=yes
LockPersonality=yes
RestrictRealtime=yes
UMask=0077
KillMode=control-group
TimeoutStopSec=10
"
    );
    let rule = format!(
        "// {user} starts and stops {NAME}'s services, each in its sandbox ({CLI}-service@*), and nothing else.
polkit.addRule(function (action, subject) {{
  if (action.id == \"org.freedesktop.systemd1.manage-units\" && subject.user == \"{user}\" &&
      String(action.lookup(\"unit\") || \"\").indexOf(\"{CLI}-service@\") == 0 &&
      [\"start\", \"stop\", \"restart\", \"reset-failed\", \"clean\"].indexOf(String(action.lookup(\"verb\"))) >= 0) {{
    return polkit.Result.YES;
  }}
}});
"
    );
    Ok(format!(
        "set -eu
# {NAME}'s services, each in its sandbox, started by {user}'s {CLI}d: the unit they run as, and polkit's rule.
if ! command -v pkaction >/dev/null; then
  command -v apt-get >/dev/null || {{ echo 'install polkit (polkitd), then run this again' >&2; exit 1; }}
  DEBIAN_FRONTEND=noninteractive apt-get install -y -q polkitd >/dev/null
fi
cat > /etc/systemd/system/{CLI}-service@.service <<'EOF'
{unit}EOF
mkdir -p /etc/polkit-1/rules.d
cat > /etc/polkit-1/rules.d/50-{CLI}-services.rules <<'EOF'
{rule}EOF
systemctl daemon-reload
systemctl try-reload-or-restart polkit 2>/dev/null || true
echo \"{NAME}'s services run in their sandbox here from their next start.\"
"
    ))
}

/// What the launcher is told, in `sandbox.json` in its folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Launch {
    pub name: String,
    /// Whether it gets a network of its own, reaching out only through its endpoint.
    pub own_network: bool,
    /// Ports on this machine forwarded at the same port, through its endpoint.
    pub ports: Vec<u16>,
    /// Its data's folder; none for a unit's, which systemd gives (`STATE_DIRECTORY`).
    pub data: Option<PathBuf>,
}

pub const PROGRAM: &str = "program";
/// The socket to its endpoint, in the sandbox's folder.
pub const SOCKET: &str = "socket";
/// The socket its unit's output goes to, which its endpoint copies to its own.
pub const LOG: &str = "log";
/// Its data from before it had a sandbox, which the launcher moves into its own folder once.
pub const BEFORE: &str = "data-before";

/// Puts `program`, `launcher` (this program), the token and what the launcher is told in the sandbox's folder for the
/// service `name`, and, for a unit, its data from before it had one.
pub fn prepare(name: &str, program: &Path, launcher: &Path, token: &str, launch: &Launch) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = dir(name);
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(root(), std::fs::Permissions::from_mode(0o700))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))?;
    for (from, to) in [(launcher, format!("{CLI}d")), (program, PROGRAM.to_string())] {
        let to = dir.join(to);
        let _ = std::fs::remove_file(&to);
        if std::fs::hard_link(from, &to).is_err() {
            let new = to.with_extension("new");
            std::fs::copy(from, &new).with_context(|| format!("copying {}", from.display()))?;
            std::fs::rename(&new, &to)?;
        }
    }
    let token_file = dir.join(crate::api::LOCAL_TOKEN);
    std::fs::write(&token_file, token)?;
    std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o644))?;
    std::fs::write(dir.join("sandbox.json"), serde_json::to_vec_pretty(launch)?)?;
    // A unit's data is systemd's (its own user's): what the service kept before is handed over once, readable to it.
    let before = crate::home().join(name);
    if launch.data.is_none() && before.is_dir() && std::fs::read_dir(&before)?.next().is_some() && !dir.join(BEFORE).exists() {
        std::fs::rename(&before, dir.join(BEFORE)).with_context(|| format!("moving {} into its sandbox", before.display()))?;
        readable(&dir.join(BEFORE))?;
    }
    Ok(dir)
}

/// Makes everything under `path` readable to its unit's user, whose folder alone shows it.
fn readable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    let mode = meta.permissions().mode() | if meta.is_dir() { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    if meta.is_dir() {
        for entry in std::fs::read_dir(path)? {
            readable(&entry?.path())?;
        }
    }
    Ok(())
}

/// Runs `systemctl` on a unit of ours, never asking for a password.
pub fn systemctl(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("systemctl").arg("--no-ask-password").args(args).output().context("running systemctl")?;
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() && args[0] != "is-active" {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if why.contains("Access denied") || why.contains("authentication required") || why.contains("Interactive authentication") {
            bail!("this machine doesn’t let {CLI}d start it in its sandbox: run `{CLI} services sandbox | sudo sh` there");
        }
        bail!("systemctl {}: {why}", args.join(" "));
    }
    Ok(said)
}

/// The Seatbelt profile for a service on a Mac: everything as the user, but the network only to its endpoint (and
/// the ports here it declares), unless it declares any host; and, unless it runs as the user, no files of theirs
/// but its own folder, `data`, and its `program`.
pub fn seatbelt(declared: &Declared, endpoint: u16, data: &Path, token: &Path, program: &Path) -> String {
    let quoted = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""));
    let mut profile = String::from("(version 1)\n(allow default)\n");
    if !declared.any_host() {
        profile.push_str("(deny network*)\n");
        for port in std::iter::once(endpoint).chain(declared.local_ports()) {
            profile.push_str(&format!("(allow network-outbound (remote ip \"localhost:{port}\"))\n"));
        }
    }
    if !declared.user {
        profile.push_str("(deny file-read* file-write* (subpath \"/Users\") (subpath \"/Volumes\") (subpath \"/private/var/folders\"))\n");
        profile.push_str(&format!("(allow file-read* file-write* (subpath {}))\n", quoted(data)));
        profile.push_str(&format!("(allow file-read* (literal {}) (literal {}))\n", quoted(token), quoted(program)));
        // Its program's folder, where a Mac app keeps it beside what it's linked with.
        if let Some(parent) = program.parent() {
            profile.push_str(&format!("(allow file-read* (subpath {}))\n", quoted(parent)));
        }
    }
    profile
}

/// The launcher: given its folder, gives the program in it a network of its own (if it's to have one), moves its data
/// from before in, forwards a port to its endpoint, starts it, and stops it when asked or when its endpoint is gone.
#[cfg(target_os = "linux")]
pub fn launch(dir: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let spec: Launch = serde_json::from_slice(&std::fs::read(dir.join("sandbox.json")).context("its sandbox.json")?)?;
    // SAFETY: plain prctl calls on this process.
    unsafe {
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
    if spec.own_network {
        own_network().context("giving it a network of its own")?;
    }
    let data = match &spec.data {
        Some(d) => d.clone(),
        None => std::env::var_os("STATE_DIRECTORY").map(PathBuf::from).context("its unit gives it no folder (STATE_DIRECTORY)")?,
    };
    std::fs::create_dir_all(&data)?;
    move_in(&dir.join(BEFORE), &data)?;
    let socket = dir.join(SOCKET);
    let api = std::net::TcpListener::bind("127.0.0.1:0")?;
    let at = api.local_addr()?;
    forward(api, socket.clone(), None);
    for port in &spec.ports {
        match std::net::TcpListener::bind(("127.0.0.1", *port)) {
            Ok(l) => forward(l, socket.clone(), Some(*port)),
            Err(err) => eprintln!("forwarding port {port}: {err}"),
        }
    }
    let proxy = format!("http://{at}");
    let mut command = std::process::Command::new(dir.join(PROGRAM));
    command
        .envs(crate::services::program_env(&spec.name, at, dir, &data))
        .env("TMPDIR", std::env::temp_dir())
        .current_dir(&data);
    for var in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        command.env(var, &proxy);
    }
    command.env("NO_PROXY", "localhost,127.0.0.1").env("no_proxy", "localhost,127.0.0.1");
    if spec.data.is_none() {
        command.env("HOME", &data);
    }
    if let Ok(log) = std::os::unix::net::UnixStream::connect(dir.join(LOG)) {
        let err = log.try_clone()?;
        command.stdout(std::process::Stdio::from(std::os::fd::OwnedFd::from(log))).stderr(std::process::Stdio::from(std::os::fd::OwnedFd::from(err)));
    }
    // SAFETY: only an async-signal-safe call, between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    extern "C" fn asked(_: libc::c_int) {
        ASKED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    // SAFETY: the handler only stores to an atomic.
    unsafe { libc::signal(libc::SIGTERM, asked as *const () as libc::sighandler_t) };
    let mut child = command.spawn().with_context(|| format!("starting {}", spec.name))?;
    let mut checked = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("{} stopped ({status})", spec.name);
        }
        let gone = checked.elapsed() > Duration::from_secs(2) && {
            checked = std::time::Instant::now();
            std::os::unix::net::UnixStream::connect(&socket).is_err()
        };
        if ASKED.load(std::sync::atomic::Ordering::SeqCst) || gone {
            // SAFETY: a signal to a child of this process, which it hasn't reaped.
            unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
            for _ in 0..100 {
                if child.try_wait()?.is_some() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(target_os = "linux")]
static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Puts this process in a user and network namespace of its own, as the same user, with only its loopback, up.
/// Called before it starts a thread.
#[cfg(target_os = "linux")]
fn own_network() -> Result<()> {
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    // SAFETY: unshare on a process with one thread.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } != 0 {
        bail!("this machine doesn’t let a program have a network of its own: {}", std::io::Error::last_os_error());
    }
    std::fs::write("/proc/self/setgroups", "deny")?;
    std::fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    std::fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    // SAFETY: an ioctl on a socket this opens and closes, with a zeroed ifreq naming lo.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        ensure!(fd >= 0, "a socket: {}", std::io::Error::last_os_error());
        let mut req: libc::ifreq = std::mem::zeroed();
        for (i, b) in b"lo".iter().enumerate() {
            req.ifr_name[i] = *b as libc::c_char;
        }
        let got = libc::ioctl(fd, libc::SIOCGIFFLAGS, &mut req);
        req.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
        let set = libc::ioctl(fd, libc::SIOCSIFFLAGS, &req);
        let err = std::io::Error::last_os_error();
        libc::close(fd);
        ensure!(got == 0 && set == 0, "bringing its loopback up: {err}");
    }
    Ok(())
}

/// Moves its data from before (`from`, read-only) into `to`, once, while `to` is empty: copied beside it, then moved
/// in, so a copy cut short is done again.
#[cfg(target_os = "linux")]
fn move_in(from: &Path, to: &Path) -> Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    let copying = to.with_file_name(format!(".{}-moving", to.file_name().unwrap_or_default().to_string_lossy()));
    let empty = std::fs::read_dir(to)?.next().is_none();
    if !empty {
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(&copying);
    let copy = || -> Result<()> {
        copy_tree(from, &copying)?;
        for entry in std::fs::read_dir(&copying)? {
            let entry = entry?;
            std::fs::rename(entry.path(), to.join(entry.file_name()))?;
        }
        Ok(())
    };
    // Its own folder's parent may not be writable (systemd's): copied straight in, then.
    if copy().is_err() {
        let _ = std::fs::remove_dir_all(&copying);
        copy_tree(from, to)?;
    }
    let _ = std::fs::remove_dir_all(&copying);
    eprintln!("moved {} into its sandbox", from.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if kind.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// Forwards each connection to `listener` to the endpoint's socket: as it is, or, for a port here, through its proxy.
#[cfg(target_os = "linux")]
fn forward(listener: std::net::TcpListener, socket: PathBuf, port: Option<u16>) {
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let socket = socket.clone();
            std::thread::spawn(move || {
                let Ok(mut there) = std::os::unix::net::UnixStream::connect(&socket) else { return };
                if let Some(port) = port {
                    use std::io::{Read, Write};
                    if write!(there, "CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").is_err() {
                        return;
                    }
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                        if there.read(&mut byte).unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    if !head.starts_with(b"HTTP/1.1 200") {
                        return;
                    }
                }
                let (Ok(mut a), Ok(mut b)) = (conn.try_clone(), there.try_clone()) else { return };
                let mut conn = conn;
                let up = std::thread::spawn(move || {
                    let _ = std::io::copy(&mut a, &mut b);
                    let _ = b.shutdown(std::net::Shutdown::Write);
                });
                let _ = std::io::copy(&mut there, &mut conn);
                let _ = conn.shutdown(std::net::Shutdown::Write);
                let _ = up.join();
            });
        }
    });
}

/// Answers a program's `CONNECT host:port`: a tunnel to it, if its app declares it, and it isn't this machine unless
/// declared by its address. Anything else is refused.
pub async fn connect(declared: Option<&Declared>, req: axum::extract::Request) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let refuse = |code: StatusCode, why: String| (code, why).into_response();
    if req.method() != axum::http::Method::CONNECT {
        return refuse(StatusCode::NOT_FOUND, "not found".into());
    }
    let Some(declared) = declared else { return refuse(StatusCode::FORBIDDEN, "it reaches no host through its sandbox".into()) };
    let Some((host, port)) = req.uri().authority().map(|a| (a.host().to_string(), a.port_u16().unwrap_or(443))) else {
        return refuse(StatusCode::BAD_REQUEST, "CONNECT needs host:port".into());
    };
    if !declared.allows(&host, port) {
        tracing::warn!("its sandbox refused {host}:{port}, which its app doesn't declare");
        return refuse(StatusCode::FORBIDDEN, format!("{host} isn’t a host its app declares"));
    }
    let local = ["localhost", "127.0.0.1", "::1"].contains(&host.trim_start_matches('[').trim_end_matches(']'));
    let addrs: Vec<std::net::SocketAddr> = match tokio::net::lookup_host((host.trim_start_matches('[').trim_end_matches(']'), port)).await {
        Ok(a) => a.filter(|a| local || !(a.ip().is_loopback() || a.ip().is_unspecified())).collect(),
        Err(err) => return refuse(StatusCode::BAD_GATEWAY, format!("{host}: {err}")),
    };
    let mut stream = None;
    for addr in addrs {
        if let Ok(Ok(s)) = tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect(addr)).await {
            stream = Some(s);
            break;
        }
    }
    let Some(mut stream) = stream else { return refuse(StatusCode::BAD_GATEWAY, format!("couldn’t reach {host}:{port}")) };
    tokio::spawn(async move {
        match hyper::upgrade::on(req).await {
            Ok(up) => {
                let mut up = hyper_util::rt::TokioIo::new(up);
                let _ = tokio::io::copy_bidirectional(&mut up, &mut stream).await;
            }
            Err(err) => tracing::debug!("a tunnel to {host}: {err}"),
        }
    });
    StatusCode::OK.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        let d = Declared { network: vec!["web.whatsapp.com".into(), "*.whatsapp.net".into(), "127.0.0.1:11434".into()], user: false };
        assert!(d.allows("web.whatsapp.com", 443));
        assert!(d.allows("mmg.whatsapp.net", 443));
        assert!(!d.allows("whatsapp.net.evil.com", 443));
        assert!(!d.allows("evil.com", 443));
        assert!(d.allows("127.0.0.1", 11434));
        assert!(!d.allows("127.0.0.1", 22));
        assert_eq!(d.local_ports(), vec![11434]);
        assert!(!d.any_host());
        assert!(Declared { network: vec!["*".into()], user: true }.allows("anything.com", 22));
    }
}
