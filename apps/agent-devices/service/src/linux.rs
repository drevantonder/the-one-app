//! Linux devices: each a container running Cua's desktop image with Google Chrome, named `device-<name>`, its home
//! folder on a volume of its own, so its sign-ins survive restarts and image updates. Its server's port and token are
//! labels on the container. The container runtime is Podman on Linux and Apple's container on a Mac.
//!
//! The runtime is the machine's, shared by every network on it (a `one throwaway` beside the real one), so each container
//! carries its network's data folder as a label (`one.home`), and a network lists and touches only its own. Those made
//! before the label are claimed once, by a network whose folder was already there when it first ran with it
//! (`claimed.json`): the real one, never a new `one throwaway`.

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use super::Devices;

pub const PREFIX: &str = "device-";
pub const HOME: &str = "/home/cua";
const FIRST_PORT: u16 = 32100;

/// A Linux device, as its container runtime lists it.
#[derive(Clone, Debug, Default)]
pub struct Device {
    pub name: String,
    /// running, paused or stopped.
    pub state: String,
    pub started: Option<jiff::Timestamp>,
    /// Made to be borrowed (pool.rs).
    pub pool: bool,
    /// The image it runs.
    pub image: String,
    pub port: u16,
    pub token: String,
    /// The data folder of the network it's of, or "" if it was made before that was kept.
    pub home: String,
}

/// This network's data folder, and the containers made before `one.home` that it claimed.
static OWNER: OnceLock<(String, BTreeSet<String>)> = OnceLock::new();

/// Whether a container listed is this network's.
fn ours(d: &Device) -> bool {
    OWNER.get().is_some_and(|(home, claimed)| if d.home.is_empty() { claimed.contains(&d.name) } else { &d.home == home })
}

/// Learns which containers are this network's, claiming those without `one.home` the first time if `fresh` is false.
fn own(rt: &dyn Containers, dir: &Path, fresh: bool) -> Result<()> {
    let file = dir.join("claimed.json");
    let claimed: BTreeSet<String> = match std::fs::read(&file) {
        Ok(b) => serde_json::from_slice(&b)?,
        Err(_) => {
            let claimed = if fresh { BTreeSet::new() } else { rt.all()?.into_iter().filter(|d| d.home.is_empty()).map(|d| d.name).collect() };
            std::fs::write(&file, serde_json::to_vec(&claimed)?)?;
            if !claimed.is_empty() {
                tracing::info!("claimed the devices made before networks were told apart: {claimed:?}");
            }
            claimed
        }
    };
    let _ = OWNER.set((dir.to_string_lossy().to_string(), claimed));
    Ok(())
}

impl Device {
    pub fn local(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// What its container carries.
    fn labels(&self) -> Vec<String> {
        let home = OWNER.get().map(|(home, _)| home.as_str()).unwrap_or_default();
        let mut l = vec!["--label".into(), format!("one.port={}", self.port), "--label".into(), format!("one.token={}", self.token), "--label".into(), format!("one.home={home}")];
        if self.pool {
            l.extend(["--label".into(), "one.pool=1".into()]);
        }
        l
    }
}

/// A device's name: lowercase letters, digits and dashes.
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty() && b.len() <= 40 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// How much of the device it's on an agent device may use, offered by name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub name: &'static str,
    pub cpus: u32,
    pub gb: u32,
}

pub const SIZES: [Size; 3] = [Size { name: "small", cpus: 2, gb: 4 }, Size { name: "medium", cpus: 4, gb: 8 }, Size { name: "large", cpus: 8, gb: 16 }];

pub fn size_named(name: &str) -> Result<Size> {
    if name.is_empty() {
        return Ok(SIZES[0]);
    }
    SIZES.iter().find(|s| s.name.eq_ignore_ascii_case(name)).copied().ok_or_else(|| anyhow!("no size {name:?}: there's {}", SIZES.map(|s| s.name).join(", ")))
}

impl Size {
    fn limits(&self) -> Vec<String> {
        vec!["--cpus".into(), self.cpus.to_string(), "--memory".into(), format!("{}g", self.gb), "--memory-swap".into(), format!("{}g", 2 * self.gb)]
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} ({} CPUs, {} GB)", self.name, self.cpus, self.gb)
    }
}

/// What a device may use, as its container says.
#[derive(Clone, Copy, Debug, Default)]
pub struct Limit {
    /// 0: no limit.
    pub cpus: f64,
    /// Bytes; 0: no limit.
    pub memory: i64,
    /// Started again when the device it's on restarts.
    pub restart: bool,
}

impl Limit {
    /// Names a device's limits; devices made before sizes, with only a memory limit, are named by their memory.
    pub fn size(&self) -> String {
        SIZES.iter().find(|s| self.memory == (s.gb as i64) << 30 && (self.cpus == 0.0 || self.cpus == s.cpus as f64)).map(|s| s.name).unwrap_or("custom").to_string()
    }
}

/// What a device uses now: CPU in percent of one core, memory in bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub cpu: f64,
    pub memory: i64,
}

/// A size as Podman writes it, like 88.25MB.
fn bytes_of(s: &str) -> i64 {
    let s = s.trim();
    let i = s.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(s.len());
    let n: f64 = s[..i].parse().unwrap_or(0.0);
    let scale = match s[i..].to_lowercase().as_str() {
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => (1u64 << 20) as f64,
        "gib" => (1u64 << 30) as f64,
        _ => 1.0,
    };
    (n * scale) as i64
}

/// A command here, pointed at the user's own systemd, which a service has no login session for (lingering keeps it):
/// Podman and systemd-run need it.
pub fn cmd(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    if cfg!(target_os = "linux") && std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        let dir = format!("/run/user/{}", unsafe { libc::getuid() });
        if Path::new(&dir).exists() {
            c.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={dir}/bus")).env("XDG_RUNTIME_DIR", dir);
        }
    }
    c
}

/// Runs a command, its error carrying what it said on stderr: what it wrote to stdout.
pub fn command(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = cmd(program).args(args).stdin(Stdio::null()).output().map_err(|err| anyhow!("{program}: {err}"))?;
    if !out.status.success() {
        bail!("{program} {}: {}: {}", args.first().unwrap_or(&""), out.status, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// A command's failure that keeps its exit code, which some callers accept.
#[derive(Debug)]
pub struct ExitError {
    pub code: Option<i32>,
    pub said: String,
}

impl std::fmt::Display for ExitError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.code {
            Some(code) => write!(f, "exit status {code}: {}", self.said),
            None => write!(f, "killed: {}", self.said),
        }
    }
}

impl std::error::Error for ExitError {}

/// Whether tar's error is only that files changed while it read them, as a running browser's do.
pub fn tarred(r: &Result<()>) -> bool {
    match r {
        Ok(()) => true,
        Err(err) => err.downcast_ref::<ExitError>().is_some_and(|e| e.code == Some(1)),
    }
}

/// Runs `command` with `stdin`, writing what it says to `stdout`: an `ExitError` if it fails.
pub fn exec_with(mut command: Command, stdin: Option<&mut (dyn Read + Send)>, stdout: Option<&mut dyn Write>) -> Result<()> {
    command.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut err = child.stderr.take().unwrap();
    let mut out = child.stdout.take().unwrap();
    let said = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let copied = std::thread::scope(|scope| -> Result<()> {
        if let (Some(stdin), Some(mut w)) = (stdin, child.stdin.take()) {
            scope.spawn(move || {
                let _ = std::io::copy(stdin, &mut w);
            });
        }
        match stdout {
            Some(to) => std::io::copy(&mut out, to).map(|_| ())?,
            None => std::io::copy(&mut out, &mut std::io::sink()).map(|_| ())?,
        }
        Ok(())
    });
    let status = child.wait()?;
    let said = said.join().unwrap_or_default();
    copied?;
    if !status.success() {
        return Err(ExitError { code: status.code(), said: said.trim().to_string() }.into());
    }
    Ok(())
}

/// What runs devices' containers.
pub trait Containers: Send + Sync {
    /// Every device container on this machine, any network's.
    fn all(&self) -> Result<Vec<Device>>;

    /// This network's devices.
    fn list(&self) -> Result<Vec<Device>> {
        Ok(self.all()?.into_iter().filter(ours).collect())
    }
    /// Makes the device's container and home volume from the image, and starts it.
    fn run(&self, d: &Device, size: Size, image: &str) -> Result<()>;
    /// Starts a device; whether it should be running, which revive applies after a restart, is lifecycle.rs's.
    fn start(&self, d: &Device) -> Result<()>;
    fn stop(&self, d: &Device) -> Result<()>;
    fn restart(&self, d: &Device) -> Result<()>;
    /// Deletes the container and its volumes.
    fn remove(&self, d: &Device) -> Result<()>;
    /// Freezes a device's processes: it keeps its memory but uses no CPU until unpaused, which takes a moment.
    fn pause(&self, d: &Device) -> Result<()>;
    fn unpause(&self, d: &Device) -> Result<()>;
    /// A command run in a running device, as root.
    fn exec(&self, d: &Device, args: &[&str]) -> Command;
    fn limits(&self, all: &[Device]) -> Result<HashMap<String, Limit>>;
    fn resize(&self, d: &Device, size: Size) -> Result<()>;
    fn stats(&self) -> Result<HashMap<String, Usage>>;
    fn has_image(&self, image: &str) -> bool;
    /// Builds an image from the Containerfile in dir.
    fn build(&self, image: &str, dir: &Path) -> Result<()>;
    fn images(&self) -> Result<Vec<String>>;
    fn remove_image(&self, image: &str) -> Result<()>;
    /// Whether it can pause devices (Apple's container can't).
    fn pauses(&self) -> bool {
        true
    }
}

/// Deletes the containers of the network whose data folder is `dir`, as when a `one throwaway` is deleted.
pub fn delete_all(dir: &Path) -> Result<()> {
    let rt: Box<dyn Containers> = if cfg!(target_os = "macos") { Box::new(Apple::new(dir)?) } else { Box::new(Podman::new()?) };
    let home = dir.to_string_lossy();
    for d in rt.all()?.into_iter().filter(|d| d.home == home) {
        rt.remove(&d)?;
    }
    Ok(())
}

pub fn runtime(dir: &Path) -> Result<Box<dyn Containers>> {
    let fresh = std::fs::read_dir(dir).map_or(true, |mut d| d.next().is_none());
    let rt: Box<dyn Containers> = if cfg!(target_os = "macos") { Box::new(Apple::new(dir)?) } else { Box::new(Podman::new()?) };
    own(rt.as_ref(), dir, fresh)?;
    Ok(rt)
}

/// Whether a program is on the PATH.
pub fn found(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(program).is_file()))
}

/// Runs devices as rootless Podman containers, in the user's own systemd instance rather than this service, so
/// restarting it leaves them running. They have no restart policy: after a reboot this service starts those that
/// should run (revive), as many as have room.
struct Podman;

impl Podman {
    fn new() -> Result<Podman> {
        if !found("podman") {
            bail!("needs Podman");
        }
        let dir = format!("/run/user/{}", unsafe { libc::getuid() });
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() && !Path::new(&dir).exists() {
            bail!("Podman needs {dir}: run loginctl enable-linger {}", std::env::var("USER").unwrap_or_default());
        }
        // Containers made before revive have --restart=always, which Podman can't change, and podman-restart would
        // start them at boot whether or not there's room: revive does instead.
        if let Err(err) = command("systemctl", &["--user", "disable", "podman-restart.service"]) {
            tracing::info!("disabling podman-restart: {err:#}");
        }
        Ok(Podman)
    }
}

fn podman(args: &[&str]) -> Result<Vec<u8>> {
    command("podman", args)
}

impl Containers for Podman {
    fn all(&self) -> Result<Vec<Device>> {
        let out = podman(&["ps", "--all", "--filter", "label=one.token", "--format", "json"])?;
        let raw: Vec<Value> = serde_json::from_slice(&out).unwrap_or_default();
        let mut all = Vec::new();
        for r in raw {
            let Some(name) = r["Names"][0].as_str().and_then(|n| n.strip_prefix(PREFIX)) else { continue };
            let label = |k: &str| r["Labels"][k].as_str().unwrap_or_default().to_string();
            let state = r["State"].as_str().unwrap_or_default();
            all.push(Device {
                name: name.to_string(),
                state: if state == "running" || state == "paused" { state.to_string() } else { "stopped".into() },
                started: r["StartedAt"].as_i64().filter(|s| *s > 0).and_then(|s| jiff::Timestamp::from_second(s).ok()),
                pool: !label("one.pool").is_empty(),
                image: r["Image"].as_str().unwrap_or_default().to_string(),
                port: label("one.port").parse().unwrap_or(0),
                token: label("one.token"),
                home: label("one.home"),
            });
        }
        Ok(all)
    }

    fn run(&self, d: &Device, size: Size, image: &str) -> Result<()> {
        let c = format!("{PREFIX}{}", d.name);
        let mut args: Vec<String> = vec!["run".into(), "--detach".into(), "--name".into(), c.clone()];
        args.extend(d.labels());
        args.extend(size.limits());
        args.extend([
            "--env".into(),
            format!("CUA_ENV_TOKEN={}", d.token),
            // The image says HOME=/root, which its user inherits; there Chrome can't keep crash reports, and won't
            // start.
            "--env".into(),
            format!("HOME={HOME}"),
            "--publish".into(),
            format!("127.0.0.1:{}:3211", d.port),
            // A new named volume starts as a copy of the image's home folder.
            "--volume".into(),
            format!("{c}-home:{HOME}"),
            "--shm-size=1g".into(),
            image.into(),
        ]);
        podman(&args.iter().map(String::as_str).collect::<Vec<_>>()).map(|_| ())
    }

    fn has_image(&self, image: &str) -> bool {
        podman(&["image", "inspect", image]).is_ok()
    }

    /// Pulls Cua's image again first, and then deletes the one it replaced. Docker's format keeps Cua's health check,
    /// which OCI's drops.
    fn build(&self, image: &str, dir: &Path) -> Result<()> {
        podman(&["build", "--pull=always", "--format", "docker", "--tag", image, &dir.to_string_lossy()])?;
        if let Err(err) = podman(&["image", "prune", "--force", "--filter", "label=org.opencontainers.image.source=https://github.com/trycua/cua"]) {
            tracing::info!("{err:#}");
        }
        Ok(())
    }

    fn images(&self) -> Result<Vec<String>> {
        let out = podman(&["images", "--format", "{{.Repository}}:{{.Tag}}"])?;
        Ok(String::from_utf8_lossy(&out).split_whitespace().map(str::to_string).collect())
    }

    fn remove_image(&self, image: &str) -> Result<()> {
        podman(&["image", "rm", image]).map(|_| ())
    }

    fn start(&self, d: &Device) -> Result<()> {
        podman(&["start", &format!("{PREFIX}{}", d.name)]).map(|_| ())
    }

    fn stop(&self, d: &Device) -> Result<()> {
        podman(&["stop", "--time", "10", &format!("{PREFIX}{}", d.name)]).map(|_| ())
    }

    fn restart(&self, d: &Device) -> Result<()> {
        podman(&["restart", "--time", "10", &format!("{PREFIX}{}", d.name)]).map(|_| ())
    }

    fn remove(&self, d: &Device) -> Result<()> {
        let c = format!("{PREFIX}{}", d.name);
        let out = podman(&["inspect", "--format", "{{range .Mounts}}{{.Name}} {{end}}", &c])?;
        podman(&["rm", "--force", "--time", "10", &c])?;
        for v in String::from_utf8_lossy(&out).split_whitespace() {
            podman(&["volume", "rm", v])?;
        }
        Ok(())
    }

    fn pause(&self, d: &Device) -> Result<()> {
        podman(&["pause", &format!("{PREFIX}{}", d.name)]).map(|_| ())
    }

    fn unpause(&self, d: &Device) -> Result<()> {
        podman(&["unpause", &format!("{PREFIX}{}", d.name)]).map(|_| ())
    }

    fn exec(&self, d: &Device, args: &[&str]) -> Command {
        let mut c = cmd("podman");
        c.args(["exec", "--interactive", "--user", "root", &format!("{PREFIX}{}", d.name)]).args(args);
        c
    }

    fn limits(&self, all: &[Device]) -> Result<HashMap<String, Limit>> {
        let mut out = HashMap::new();
        if all.is_empty() {
            return Ok(out);
        }
        let names: Vec<String> = all.iter().map(|d| format!("{PREFIX}{}", d.name)).collect();
        let mut args = vec!["inspect", "--format", "json"];
        args.extend(names.iter().map(String::as_str));
        let got: Vec<Value> = serde_json::from_slice(&podman(&args)?)?;
        for g in got {
            let name = g["Name"].as_str().unwrap_or_default().trim_start_matches('/');
            let h = &g["HostConfig"];
            out.insert(
                name.strip_prefix(PREFIX).unwrap_or(name).to_string(),
                Limit { cpus: h["NanoCpus"].as_f64().unwrap_or(0.0) / 1e9, memory: h["Memory"].as_i64().unwrap_or(0), restart: h["RestartPolicy"]["Name"] == "always" },
            );
        }
        Ok(out)
    }

    fn resize(&self, d: &Device, size: Size) -> Result<()> {
        let mut args = vec!["update".to_string()];
        args.extend(size.limits());
        args.push(format!("{PREFIX}{}", d.name));
        podman(&args.iter().map(String::as_str).collect::<Vec<_>>()).map(|_| ())
    }

    fn stats(&self) -> Result<HashMap<String, Usage>> {
        let raw: Vec<Value> = serde_json::from_slice(&podman(&["stats", "--no-stream", "--format", "json"])?)?;
        let mut all = HashMap::new();
        for r in raw {
            let Some(name) = r["name"].as_str().and_then(|n| n.strip_prefix(PREFIX)) else { continue };
            let cpu = r["cpu_percent"].as_str().unwrap_or_default().trim_end_matches('%').trim().parse().unwrap_or(0.0);
            // "88.25MB / 4.295GB"
            let mem = r["mem_usage"].as_str().unwrap_or_default().split(" /").next().unwrap_or_default();
            all.insert(name.to_string(), Usage { cpu, memory: bytes_of(mem) });
        }
        Ok(all)
    }
}

/// Runs devices with Apple's container (github.com/apple/container), each in a light VM of its own, on a Mac. A
/// device's server is reached through a Unix socket the VM publishes on the Mac, not over the VM's network or a
/// published port: macOS treats that network as the local network, which a background service isn't allowed to reach.
/// A small bridge in the device joins the socket to its server, and this serves the socket on 127.0.0.1:<port>.
/// Apple's container can't pause or resize a container, and has no restart policy: revive starts those that should
/// run.
struct Apple {
    /// Sockets, and which devices were stopped.
    dir: PathBuf,
    forwards: Mutex<HashMap<String, ()>>,
    /// The last CPU reading of each device: its microseconds, and when.
    cpu: Mutex<HashMap<String, (i64, Instant)>>,
}

/// The bridge's end, in the device.
const DEVICE_SOCKET: &str = "/run/one-device.sock";

/// Joins DEVICE_SOCKET to the device's server. It runs in the device, which has Python, and is started again whenever
/// the device has restarted.
const BRIDGE: &str = r#"
import os, socket, threading
path = "/run/one-device.sock"
try: os.unlink(path)
except OSError: pass
s = socket.socket(socket.AF_UNIX); s.bind(path); s.listen(64)
def pipe(a, b):
    try:
        while (d := a.recv(65536)): b.sendall(d)
    except OSError: pass
    try: b.shutdown(socket.SHUT_WR)
    except OSError: pass
while True:
    c, _ = s.accept()
    try: u = socket.create_connection(("127.0.0.1", 3211))
    except OSError: c.close(); continue
    threading.Thread(target=pipe, args=(c, u), daemon=True).start()
    threading.Thread(target=pipe, args=(u, c), daemon=True).start()
"#;

/// What Apple's container can't do.
#[derive(Debug)]
pub struct Cant;

impl std::fmt::Display for Cant {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Apple's container can't do that")
    }
}

impl std::error::Error for Cant {}

fn ctr(args: &[&str]) -> Result<Vec<u8>> {
    command("container", args)
}

impl Apple {
    fn new(dir: &Path) -> Result<Apple> {
        if !found("container") {
            bail!("needs Apple's container (brew install container; container system start)");
        }
        for d in ["sockets", "stopped"] {
            std::fs::create_dir_all(dir.join(d))?;
        }
        Ok(Apple { dir: dir.to_path_buf(), forwards: Mutex::new(HashMap::new()), cpu: Mutex::new(HashMap::new()) })
    }

    fn socket(&self, name: &str) -> PathBuf {
        self.dir.join("sockets").join(format!("{name}.sock"))
    }

    fn stopped(&self, name: &str) -> PathBuf {
        self.dir.join("stopped").join(name)
    }

    fn containers(&self) -> Result<Vec<Value>> {
        Ok(serde_json::from_slice(&ctr(&["list", "--all", "--format", "json"])?)?)
    }

    /// Starts the bridge in a running device.
    fn bridge(name: &str) -> Result<()> {
        ctr(&["exec", "--detach", "--user", "root", &format!("{PREFIX}{name}"), "python3", "-c", BRIDGE]).map(|_| ())
    }

    /// Connects to a device's server through its socket, starting the bridge once if it isn't answering, as after the
    /// device restarted.
    fn dial(socket: &Path, name: &str) -> Result<UnixStream> {
        if let Ok(c) = UnixStream::connect(socket) {
            return Ok(c);
        }
        Self::bridge(name)?;
        std::thread::sleep(Duration::from_millis(500));
        Ok(UnixStream::connect(socket)?)
    }

    /// Serves a device on 127.0.0.1:port.
    fn forward(&self, name: &str, port: u16) {
        let mut forwards = self.forwards.lock().unwrap();
        if forwards.contains_key(name) || port == 0 {
            return;
        }
        let ln = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(ln) => ln,
            Err(err) => {
                tracing::warn!("serving {name}: {err}");
                return;
            }
        };
        forwards.insert(name.to_string(), ());
        let (name, socket) = (name.to_string(), self.socket(name));
        std::thread::spawn(move || {
            for conn in ln.incoming() {
                let Ok(conn) = conn else { continue };
                let (name, socket) = (name.clone(), socket.clone());
                std::thread::spawn(move || {
                    let backend = match Self::dial(&socket, &name) {
                        Ok(b) => b,
                        Err(err) => {
                            tracing::warn!("device {name}: {err:#}");
                            return;
                        }
                    };
                    join(conn, backend);
                });
            }
        });
    }
}

/// Copies both ways between a TCP connection and a Unix one until each side ends.
fn join(tcp: TcpStream, unix: UnixStream) {
    let (Ok(mut tcp_r), Ok(mut unix_w)) = (tcp.try_clone(), unix.try_clone()) else { return };
    let up = std::thread::spawn(move || {
        let _ = std::io::copy(&mut tcp_r, &mut unix_w);
        let _ = unix_w.shutdown(std::net::Shutdown::Write);
    });
    let (mut unix_r, mut tcp_w) = (unix, tcp);
    let _ = std::io::copy(&mut unix_r, &mut tcp_w);
    let _ = tcp_w.shutdown(std::net::Shutdown::Write);
    let _ = up.join();
}

impl Containers for Apple {
    fn all(&self) -> Result<Vec<Device>> {
        let mut out = Vec::new();
        for c in self.containers()? {
            let conf = &c["configuration"];
            let l = &conf["labels"];
            let Some(name) = conf["id"].as_str().and_then(|i| i.strip_prefix(PREFIX)) else { continue };
            let token = l["one.token"].as_str().unwrap_or_default();
            if token.is_empty() {
                continue;
            }
            let d = Device {
                name: name.to_string(),
                state: if c["status"]["state"] == "running" { "running".into() } else { "stopped".into() },
                started: c["status"]["startedDate"].as_str().and_then(|s| s.parse().ok()),
                pool: l["one.pool"].as_str().is_some_and(|p| !p.is_empty()),
                image: conf["image"]["reference"].as_str().unwrap_or_default().to_string(),
                port: l["one.port"].as_str().and_then(|p| p.parse().ok()).unwrap_or(0),
                token: token.to_string(),
                home: l["one.home"].as_str().unwrap_or_default().to_string(),
            };
            if ours(&d) {
                self.forward(&d.name, d.port); // once the runtime answers, which at login can take a while
            }
            out.push(d);
        }
        Ok(out)
    }

    fn run(&self, d: &Device, size: Size, image: &str) -> Result<()> {
        let (c, vol) = (format!("{PREFIX}{}", d.name), format!("{PREFIX}{}-home", d.name));
        ctr(&["volume", "create", &vol])?;
        // A new volume is empty: the image's home folder is copied in first.
        ctr(&["run", "--rm", "--entrypoint", "/bin/sh", "--volume", &format!("{vol}:/mnt/home"), image, "-c", &format!("cp -a {HOME}/. /mnt/home/ && rm -rf /mnt/home/lost+found && chown 1000:1000 /mnt/home")])?;
        let sock = self.socket(&d.name);
        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_file(self.stopped(&d.name));
        let mut args: Vec<String> = vec!["run".into(), "--detach".into(), "--name".into(), c];
        args.extend(d.labels());
        args.extend([
            "--env".into(),
            format!("CUA_ENV_TOKEN={}", d.token),
            "--env".into(),
            format!("HOME={HOME}"),
            "--volume".into(),
            format!("{vol}:{HOME}"),
            "--publish-socket".into(),
            format!("{}:{DEVICE_SOCKET}", sock.display()),
            "--shm-size".into(),
            "1G".into(),
            "--memory".into(),
            format!("{}G", size.gb),
            "--cpus".into(),
            size.cpus.to_string(),
            image.into(),
        ]);
        ctr(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        self.forward(&d.name, d.port);
        Self::bridge(&d.name)
    }

    fn start(&self, d: &Device) -> Result<()> {
        ctr(&["start", &format!("{PREFIX}{}", d.name)])?;
        Self::bridge(&d.name)
    }

    fn stop(&self, d: &Device) -> Result<()> {
        ctr(&["stop", &format!("{PREFIX}{}", d.name)]).map(|_| ())
    }

    fn restart(&self, d: &Device) -> Result<()> {
        ctr(&["stop", &format!("{PREFIX}{}", d.name)])?;
        ctr(&["start", &format!("{PREFIX}{}", d.name)])?;
        Self::bridge(&d.name)
    }

    fn remove(&self, d: &Device) -> Result<()> {
        // Its forward stays bound until this program restarts, for a device made again with the same name and port.
        let gone = |r: Result<Vec<u8>>| match r {
            Err(err) if !format!("{err:#}").contains("not found") => Err(err),
            _ => Ok(()),
        };
        gone(ctr(&["delete", "--force", &format!("{PREFIX}{}", d.name)]))?;
        gone(ctr(&["volume", "delete", &format!("{PREFIX}{}-home", d.name)]))?;
        let _ = std::fs::remove_file(self.socket(&d.name));
        let _ = std::fs::remove_file(self.stopped(&d.name));
        Ok(())
    }

    fn has_image(&self, image: &str) -> bool {
        ctr(&["image", "inspect", image]).is_ok()
    }

    /// Builds with container's builder, a VM of its own, stopped once the image is built: it holds memory the Mac's
    /// person needs.
    fn build(&self, image: &str, dir: &Path) -> Result<()> {
        let built = ctr(&["build", "--tag", image, &dir.to_string_lossy()]);
        if let Err(err) = ctr(&["builder", "stop"]) {
            tracing::info!("stopping container's builder: {err:#}");
        }
        built.map(|_| ())
    }

    fn images(&self) -> Result<Vec<String>> {
        let all: Vec<Value> = serde_json::from_slice(&ctr(&["image", "list", "--format", "json"])?)?;
        Ok(all.iter().filter_map(|i| i["reference"].as_str().map(str::to_string)).collect())
    }

    fn remove_image(&self, image: &str) -> Result<()> {
        ctr(&["image", "delete", image]).map(|_| ())
    }

    fn pause(&self, _: &Device) -> Result<()> {
        Err(Cant.into())
    }

    fn unpause(&self, _: &Device) -> Result<()> {
        Err(Cant.into())
    }

    fn pauses(&self) -> bool {
        false
    }

    fn exec(&self, d: &Device, args: &[&str]) -> Command {
        let mut c = cmd("container");
        c.args(["exec", "--interactive", "--user", "root", &format!("{PREFIX}{}", d.name)]).args(args);
        c
    }

    fn limits(&self, _: &[Device]) -> Result<HashMap<String, Limit>> {
        let mut out = HashMap::new();
        for c in self.containers()? {
            let conf = &c["configuration"];
            if let Some(name) = conf["id"].as_str().and_then(|i| i.strip_prefix(PREFIX)) {
                let r = &conf["resources"];
                out.insert(name.to_string(), Limit { cpus: r["cpus"].as_f64().unwrap_or(0.0), memory: r["memoryInBytes"].as_i64().unwrap_or(0), restart: !self.stopped(name).exists() });
            }
        }
        Ok(out)
    }

    fn resize(&self, _: &Device, _: Size) -> Result<()> {
        Err(anyhow!("{Cant}: back it up and restore it with another size"))
    }

    /// Reads memory and, since the last reading, CPU.
    fn stats(&self) -> Result<HashMap<String, Usage>> {
        let raw: Vec<Value> = serde_json::from_slice(&ctr(&["stats", "--no-stream", "--format", "json"])?)?;
        let mut cpu = self.cpu.lock().unwrap();
        let now = Instant::now();
        let mut all = HashMap::new();
        for r in raw {
            let Some(name) = r["id"].as_str().and_then(|i| i.strip_prefix(PREFIX)) else { continue };
            let usec = r["cpuUsageUsec"].as_i64().unwrap_or(0);
            let mut u = Usage { cpu: 0.0, memory: r["memoryUsageBytes"].as_i64().unwrap_or(0) };
            if let Some((last, at)) = cpu.get(name)
                && now > *at
                && usec >= *last
            {
                u.cpu = (usec - last) as f64 / now.duration_since(*at).as_micros() as f64 * 100.0;
            }
            cpu.insert(name.to_string(), (usec, now));
            all.insert(name.to_string(), u);
        }
        Ok(all)
    }
}

/// Devices run Cua's image with Google Chrome in place of its Chromium, so the person can sign in to the browser as
/// their Google account. Chrome's terms don't let us pass it on, so each device builds the image itself, downloading
/// Chrome from Google, and builds it again each week for Chrome's security fixes. Cua's image turns browser sign-in
/// and sync off for Chromium; Chrome's policies here leave them to the person.
///
/// Cua's viewer refuses to be shown in a frame (frame-ancestors 'none', built into cua-spacesd); apps show it inside
/// their own screens, and its link's ticket is what lets someone watch, so the recipe lets any page frame it, a
/// same-length replacement in the binary. If Cua changes that string, the build fails rather than leave the viewer
/// unframeable. Its bytes name the image (`image_for`), so they're the Go service's to the byte.
const RECIPE: &str = concat!(
    "FROM ghcr.io/trycua/linux:24.04\n",
    "RUN f=/usr/local/bin/cua-spacesd && grep -q \"frame-ancestors 'none'\" $f \\\n",
    " && perl -pi -e \"s/frame-ancestors 'none'/frame-ancestors *     /g\" $f \\\n",
    " && ! grep -q \"frame-ancestors 'none'\" $f\n",
    "RUN curl -fsSLo /tmp/chrome.deb https://dl.google.com/linux/direct/google-chrome-stable_current_$(dpkg --print-architecture).deb \\\n",
    " && apt-get update && apt-get install -y /tmp/chrome.deb \\\n",
    " && apt-get purge -y chromium chromium-common && apt-get autoremove -y \\\n",
    " && rm -rf /tmp/chrome.deb /var/lib/apt/lists/* /etc/apt/sources.list.d/google-chrome.list /etc/chromium /etc/chromium.d \\\n",
    " && mkdir -p /etc/opt/chrome/policies/managed \\\n",
    " && echo '{\"BrowserSignin\":1,\"PromotionsEnabled\":false,\"DefaultBrowserSettingEnabled\":false,\"MetricsReportingEnabled\":false,\"PrivacySandboxPromptEnabled\":false,\"TranslateEnabled\":false,\"CommandLineFlagSecurityWarningsEnabled\":false}' \\\n",
    "  > /etc/opt/chrome/policies/managed/one.json\n",
);

const IMAGE_NAME: &str = "localhost/one-agent-device";

/// The week's image: a new recipe or a new week is a new one.
pub fn image_for(now: jiff::Timestamp) -> String {
    use sha2::Digest;
    let sum = sha2::Sha256::digest(RECIPE.as_bytes());
    let week = now.to_zoned(jiff::tz::TimeZone::system()).date().iso_week_date();
    format!("{IMAGE_NAME}:{}-{}-{:02}", data_encoding::HEXLOWER.encode(&sum[..4]), week.year(), week.week())
}

impl Devices {
    pub(super) fn rt(&self) -> Result<&dyn Containers> {
        self.rt.as_deref().ok_or_else(|| anyhow!("this device runs no Linux agent devices: it has no Podman (or, on a Mac, Apple's container)"))
    }

    /// The Linux devices here.
    pub(super) fn list(&self) -> Result<Vec<Device>> {
        match &self.rt {
            Some(rt) => rt.list(),
            None => Ok(Vec::new()),
        }
    }

    pub(super) fn limits(&self, all: &[Device]) -> Result<HashMap<String, Limit>> {
        match &self.rt {
            Some(rt) => rt.limits(all),
            None => Ok(HashMap::new()),
        }
    }

    pub(super) fn stats(&self) -> HashMap<String, Usage> {
        self.rt.as_ref().and_then(|rt| rt.stats().ok()).unwrap_or_default()
    }

    /// The device named, or with no name the only one.
    pub(super) fn find(&self, name: &str) -> Result<Device> {
        pick(&self.list()?, name)
    }

    /// Runs a command in a running device, as root, with stdin, writing what it says to stdout.
    pub(super) fn exec(&self, d: &Device, stdin: Option<&mut (dyn Read + Send)>, stdout: Option<&mut dyn Write>, args: &[&str]) -> Result<()> {
        exec_with(self.rt()?.exec(d, args), stdin, stdout)
    }

    /// Runs a script in a running device, as root.
    pub(super) fn sh(&self, d: &Device, script: &str) -> Result<()> {
        self.exec(d, Some(&mut script.as_bytes()), None, &["sh", "-s"])
    }

    /// Makes a device and starts it, waiting for its server to answer if `wait`.
    pub(super) fn create(&self, name: &str, size: Size, pool: bool, wait: bool) -> Result<Device> {
        if !valid_name(name) {
            bail!("a device's name is lowercase letters, digits and dashes");
        }
        let rt = self.rt()?;
        let image = self.ensure_image()?;
        let _creating = self.creating.lock().unwrap();
        let all = rt.list()?;
        if pick(&all, name).is_ok() {
            bail!("device {name} already exists");
        }
        let port = all.iter().map(|d| d.port + 1).max().unwrap_or(FIRST_PORT).max(FIRST_PORT);
        let d = Device { name: name.to_string(), state: "running".into(), pool, port, token: super::random_hex(24), ..Default::default() };
        if let Err(err) = rt.run(&d, size, &image) {
            let _ = self.remove_device(&d);
            return Err(err);
        }
        self.want(&d.name, true);
        if wait {
            ready(&d)?;
        }
        Ok(d)
    }

    /// Starts a device, and again after the device it's on restarts, if it has room then.
    pub(super) fn start_device(&self, d: &Device) -> Result<()> {
        self.rt()?.start(d)?;
        self.want(&d.name, true);
        ready(d)
    }

    /// Stops a device until it's started again, restarts of the device it's on too.
    pub(super) fn stop_device(&self, d: &Device) -> Result<()> {
        let rt = self.rt()?;
        if d.state == "paused" {
            rt.unpause(d)?;
        }
        rt.stop(d)?;
        self.want(&d.name, false);
        Ok(())
    }

    /// Deletes a device's container and its volumes.
    pub(super) fn remove_device(&self, d: &Device) -> Result<()> {
        self.rt()?.remove(d)?;
        self.unwant(&d.name);
        if d.port > 0 {
            let _ = tailscale(&["serve", &format!("--https={}", d.port), "off"]);
        }
        Ok(())
    }

    /// The device named, unpaused if it was paused, refusing a stopped one; it counts as a call to it.
    pub(super) fn running(&self, name: &str) -> Result<Device> {
        let mut d = self.find(name)?;
        self.may_start(&d.name)?;
        match d.state.as_str() {
            "stopped" => bail!("device {} is stopped: start it first", d.name),
            "paused" => {
                self.rt()?.unpause(&d)?;
                d.state = "running".into();
            }
            _ => {}
        }
        self.touch(&d.name);
        Ok(d)
    }

    /// Pauses the running devices nobody has called or watched for a while: they keep their memory, and their
    /// browser's pages, but use no CPU. The next call unpauses one first, which takes a moment.
    pub(super) fn pause_idle(&self, all: &[Device], lent: &HashMap<String, super::Loan>) {
        const IDLE_PAUSE: Duration = Duration::from_secs(30 * 60);
        let Some(rt) = &self.rt else { return };
        if !rt.pauses() {
            return;
        }
        for d in all {
            if lent.contains_key(&d.name) || d.state != "running" || self.idle(&d.name) < IDLE_PAUSE {
                continue;
            }
            // A browser with its viewer open: a connection to its port, which the viewer keeps open while it shows it.
            if super::kinds::connected(&[d.port as i64]) {
                self.touch(&d.name);
                continue;
            }
            self.drop_conn(&d.name); // its session's requests would wait
            match rt.pause(d) {
                Ok(()) => tracing::info!("paused {}, idle for 30 minutes", d.name),
                Err(err) => tracing::warn!("pausing {}: {err:#}", d.name),
            }
        }
    }

    /// Builds this week's image if this device hasn't yet, then deletes the older ones no device uses.
    fn ensure_image(&self) -> Result<String> {
        let rt = self.rt()?;
        let _building = self.building.lock().unwrap();
        let image = image_for(super::now());
        if rt.has_image(&image) {
            return Ok(image);
        }
        let dir = std::env::temp_dir().join(format!("agent-device-image-{}", super::random_hex(6)));
        std::fs::create_dir_all(&dir)?;
        let built = (|| -> Result<()> {
            std::fs::write(dir.join("Containerfile"), RECIPE)?;
            tracing::info!("building {image}, with Chrome from Google");
            let start = Instant::now();
            rt.build(&image, &dir).map_err(|err| anyhow!("building the devices' image: {err:#}"))?;
            tracing::info!("built {image} in {}s", start.elapsed().as_secs());
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&dir);
        built?;
        for old in rt.images().unwrap_or_default() {
            if old.starts_with(&format!("{IMAGE_NAME}:")) && old != image {
                let _ = rt.remove_image(&old); // refused while a device runs on it
            }
        }
        Ok(image)
    }

    /// A link to the device's viewer that works for `minutes`, served over HTTPS on the tailnet: browsers only play its
    /// video on secure pages.
    pub(super) fn view_url(&self, d: &Device, minutes: i64) -> Result<String> {
        let port = d.port.to_string();
        tailscale(&["serve", "--bg", &format!("--https={port}"), &format!("http://127.0.0.1:{port}")])?;
        let path = viewer_path(d, minutes * 60)?;
        Ok(format!("https://{}:{port}{path}", tailnet_name()?))
    }
}

/// The device named, or with no name the only one.
pub fn pick(all: &[Device], name: &str) -> Result<Device> {
    if let Some(d) = all.iter().find(|d| d.name == name || name.is_empty() && all.len() == 1) {
        return Ok(d.clone());
    }
    let names: Vec<&str> = all.iter().map(|d| d.name.as_str()).collect();
    if all.is_empty() {
        bail!("no devices: make one with create");
    }
    if name.is_empty() {
        bail!("which device? pass device: {}", names.join(", "));
    }
    bail!("no device named {name:?}: there's {}", names.join(", "))
}

/// Waits for a device's server to answer.
pub fn ready(d: &Device) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(5))).build().into();
    let url = format!("{}/health", d.local());
    if super::wait_for(Duration::from_secs(120), Duration::from_secs(1), || agent.get(&url).call().is_ok_and(|r| r.status().as_u16() < 300)) {
        return Ok(());
    }
    bail!("device {} didn't start: it didn't answer in 2 minutes", d.name)
}

/// Runs a tailscale command. Changing what it serves is refused to an operator once anything on the machine serves a
/// Unix socket, so then it runs as root, where sudo needs no password.
pub fn tailscale(args: &[&str]) -> Result<String> {
    let said = |o: &std::process::Output| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let mut out = cmd("tailscale").args(args).output()?;
    if !out.status.success() && said(&out).contains("must be root") {
        out = cmd("sudo").args(["-n", "tailscale"]).args(args).output()?;
    }
    if !out.status.success() {
        bail!("tailscale {}: {}: {}", args[0], out.status, said(&out).trim());
    }
    Ok(said(&out))
}

/// This machine's name on the tailnet.
pub fn tailnet_name() -> Result<String> {
    let st: Value = serde_json::from_str(&tailscale(&["status", "--json"])?)?;
    Ok(st["Self"]["DNSName"].as_str().unwrap_or_default().trim_end_matches('.').to_string())
}

/// Mints a viewer ticket: a credential that only lets a browser watch and use the device (with the clipboard and the
/// home folder), for `ttl` seconds. It's SystemService.CreateViewerTicket over gRPC-Web; the two messages are small
/// enough to encode by hand.
fn viewer_path(d: &Device, ttl: i64) -> Result<String> {
    let mut ttl_msg = vec![0x08]; // Duration.seconds
    uvarint(&mut ttl_msg, ttl as u64);
    let mut msg = vec![0x0a, ttl_msg.len() as u8]; // 1: ttl
    msg.extend(ttl_msg);
    msg.extend([0x18, 0x01]); // 3: clipboard
    msg.extend([0x22, 0x01, b'~']); // 4: files_root
    let mut frame = vec![0, 0, 0, 0, msg.len() as u8];
    frame.extend(msg);
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
    let mut resp = agent
        .post(format!("{}/cua.env.v1.SystemService/CreateViewerTicket", d.local()))
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .header("authorization", format!("Bearer {}", d.token))
        .send(&frame[..])?;
    if resp.status() != 200 {
        bail!("viewer ticket: {}", resp.status());
    }
    let mut body = &resp.body_mut().read_to_vec()?[..];
    let (mut path, mut status) = (String::new(), String::new());
    while body.len() >= 5 {
        let (flag, n) = (body[0], u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize);
        if body.len() < 5 + n {
            break;
        }
        let m = &body[5..5 + n];
        body = &body[5 + n..];
        if flag & 0x80 != 0 {
            status = String::from_utf8_lossy(m).to_string(); // trailers
            continue;
        }
        path = string_field(m, 3);
    }
    if path.is_empty() {
        bail!("viewer ticket: {}", status.trim());
    }
    Ok(path)
}

fn uvarint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_uvarint(m: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, b) in m.iter().enumerate().take(10) {
        v |= ((b & 0x7f) as u64) << (7 * i);
        if b & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

/// A top-level length-delimited field of a protobuf message.
fn string_field(mut m: &[u8], field: u64) -> String {
    while !m.is_empty() {
        let Some((key, n)) = read_uvarint(m) else { return String::new() };
        m = &m[n..];
        match key & 7 {
            0 => {
                let Some((_, n)) = read_uvarint(m) else { return String::new() };
                m = &m[n..];
            }
            2 => {
                let Some((l, n)) = read_uvarint(m) else { return String::new() };
                if ((m.len() - n) as u64) < l {
                    return String::new();
                }
                let v = &m[n..n + l as usize];
                m = &m[n + l as usize..];
                if key >> 3 == field {
                    return String::from_utf8_lossy(v).to_string();
                }
            }
            _ => return String::new(),
        }
    }
    String::new()
}
