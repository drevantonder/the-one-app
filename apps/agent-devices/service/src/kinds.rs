//! Kinds: what sort of device each one is. A Linux device with a browser (linux) is a container (linux.rs); the others
//! are a virtual machine or an emulator, each run by the runtime for its kind that this machine has: Windows in QEMU on
//! Linux with KVM (windows.rs), Android in the Android SDK's emulator (android.rs), an iPhone or iPad in Xcode's
//! Simulator on a Mac (iphone.rs), and this machine's own Chrome (chrome.rs). A machine runs the kinds whose runtime
//! finds what it needs here. Another way to run a kind (Windows in a Windows machine's Hyper-V) is one more runtime in
//! `RUNTIMES`.
//!
//! A lasting device is the person's, borrowed by one agent at a time; a copy is a throwaway one made from a lasting
//! device for one agent, deleted when it's given back.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::linux::{HOME, cmd, valid_name};
use super::pool::{Loan, LoanState, POOL_PREFIX};
use super::{Caller, Devices, Input, flag, minutes, number, or, quote, text, time};

pub const LINUX: &str = "linux";

/// A runtime for one kind of device.
pub trait Kind: Sync {
    /// As tools take it: windows, android, iphone.
    fn name(&self) -> &'static str;
    /// As a person reads it.
    fn title(&self) -> &'static str;
    /// Why this machine can't run it, or "".
    fn can(&self) -> String;
    /// What input it takes, for tools' descriptions.
    fn limits(&self) -> &'static str;
    /// Whether it takes clicks and typing.
    fn input(&self) -> bool;
    /// The first port its devices take one of, every step of them; 0 for none.
    fn port(&self) -> (u16, u16);
    /// Checks that v, as adopt's input gives it, is something of this kind already here.
    fn adopt(&self, ds: &Devices, v: &mut Vm) -> Result<()>;
    /// Makes v's files: as a copy of from, or with no from, a new lasting device.
    fn make(&self, ds: &Devices, v: &mut Vm, from: Option<&Vm>) -> Result<()>;
    fn start(&self, ds: &Devices, v: &Vm) -> Result<()>;
    fn stop(&self, ds: &Devices, v: &Vm) -> Result<()>;
    /// Freezes a running device, keeping its memory but using no CPU; resume carries on. A kind that can't shuts it
    /// down instead (iPhones: Simulator can't pause).
    fn pause(&self, ds: &Devices, v: &Vm) -> Result<()>;
    fn resume(&self, ds: &Devices, v: &Vm) -> Result<()>;
    fn running(&self, ds: &Devices, v: &Vm) -> bool;
    fn remove(&self, ds: &Devices, v: &Vm) -> Result<()>;
    fn screen(&self, ds: &Devices, v: &Vm) -> Result<image::DynamicImage>;
    fn act(&self, ds: &Devices, v: &Vm, a: &Action) -> Result<()>;
    fn open(&self, ds: &Devices, v: &Vm, url: &str) -> Result<()>;
    fn install(&self, ds: &Devices, v: &Vm, app: &str) -> Result<String>;
    fn run(&self, ds: &Devices, v: &Vm, command: &str) -> Result<String>;
    /// Lets v reach port on this machine's 127.0.0.1 at the same port on its own, until it's stopped.
    fn reach(&self, ds: &Devices, v: &Vm, port: u16) -> Result<String>;
}

/// Every kind but linux, as this machine might run them.
pub static RUNTIMES: [&dyn Kind; 4] = [&super::chrome::Chrome, &super::windows::Windows, &super::android::Android, &super::iphone::Iphone];

/// A device of a kind other than linux, as its file in the service's folder says.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Vm {
    pub name: String,
    pub kind: String,
    /// The lasting device it's a throwaway copy of.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub of: String,
    /// windows: the copy's base, a folder of its lasting device's.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base: String,
    /// Its files.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    /// windows: its SSH; android: its console.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub port: u16,
    /// windows: its VNC display.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub display: u16,
    /// android: its AVD; iphone: its simulator.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cpus: u16,
    /// Memory, in GB.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gb: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paused: bool,
    #[serde(with = "time", default = "time::zero")]
    pub made: jiff::Timestamp,
}

fn is_zero(n: &u16) -> bool {
    *n == 0
}

impl Vm {
    /// Its kind's runtime.
    pub fn kind(&self) -> &'static dyn Kind {
        kind_named(&self.kind).unwrap_or(&Unknown)
    }

    /// One of its files.
    pub fn file(&self, name: &str) -> PathBuf {
        Path::new(&self.dir).join(name)
    }

    pub fn state(&self, ds: &Devices) -> &'static str {
        let k = self.kind();
        if k.can().is_empty() && k.running(ds, self) {
            return if self.paused { "paused" } else { "running" };
        }
        "stopped"
    }

    /// Whether it's a throwaway one: a copy, or one made for a loan.
    pub fn throwaway(&self) -> bool {
        !self.of.is_empty() || self.name.starts_with(POOL_PREFIX)
    }
}

/// A kind this program doesn't know, as a device's file from a newer one might say: it runs nothing.
struct Unknown;

impl Kind for Unknown {
    fn name(&self) -> &'static str {
        "unknown"
    }
    fn title(&self) -> &'static str {
        "unknown"
    }
    fn can(&self) -> String {
        "this machine doesn't know that kind".into()
    }
    fn limits(&self) -> &'static str {
        ""
    }
    fn input(&self) -> bool {
        false
    }
    fn port(&self) -> (u16, u16) {
        (0, 0)
    }
    fn adopt(&self, _: &Devices, _: &mut Vm) -> Result<()> {
        bail!("{}", self.can())
    }
    fn make(&self, _: &Devices, _: &mut Vm, _: Option<&Vm>) -> Result<()> {
        bail!("{}", self.can())
    }
    fn start(&self, _: &Devices, _: &Vm) -> Result<()> {
        bail!("{}", self.can())
    }
    fn stop(&self, _: &Devices, _: &Vm) -> Result<()> {
        Ok(())
    }
    fn pause(&self, _: &Devices, _: &Vm) -> Result<()> {
        Ok(())
    }
    fn resume(&self, _: &Devices, _: &Vm) -> Result<()> {
        bail!("{}", self.can())
    }
    fn running(&self, _: &Devices, _: &Vm) -> bool {
        false
    }
    fn remove(&self, _: &Devices, _: &Vm) -> Result<()> {
        Ok(())
    }
    fn screen(&self, _: &Devices, _: &Vm) -> Result<image::DynamicImage> {
        bail!("{}", self.can())
    }
    fn act(&self, _: &Devices, _: &Vm, _: &Action) -> Result<()> {
        bail!("{}", self.can())
    }
    fn open(&self, _: &Devices, _: &Vm, _: &str) -> Result<()> {
        bail!("{}", self.can())
    }
    fn install(&self, _: &Devices, _: &Vm, _: &str) -> Result<String> {
        bail!("{}", self.can())
    }
    fn run(&self, _: &Devices, _: &Vm, _: &str) -> Result<String> {
        bail!("{}", self.can())
    }
    fn reach(&self, _: &Devices, _: &Vm, _: u16) -> Result<String> {
        bail!("{}", self.can())
    }
}

/// Input to a device, in its screen's pixels.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Action {
    /// click, double_click, right_click, type, key, scroll, swipe.
    #[serde(default)]
    pub r#do: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub to_x: f64,
    #[serde(default)]
    pub to_y: f64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub key: String,
}

pub fn kind_named(name: &str) -> Result<&'static dyn Kind> {
    RUNTIMES.iter().copied().find(|k| k.name() == name).ok_or_else(|| anyhow!("no kind {name:?}: there's {LINUX} and {}", RUNTIMES.map(|k| k.name()).join(", ")))
}

/// Runs a command here, its error carrying what it said: what it said, stdout and stderr together.
pub fn run(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<String> {
    run_in(None, program, args)
}

/// Runs a command here with stdin.
pub fn run_in(stdin: Option<&[u8]>, program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<String> {
    run_for(None, stdin, program, args)
}

/// Runs a command here, ended and an error if it says nothing more in limit: one that hangs (adb, with a phone that
/// stopped answering) would otherwise hold its caller for as long as the caller waits.
pub fn run_within(limit: Duration, program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<String> {
    run_for(Some(limit), None, program, args)
}

/// What a command timed out in: the error of one ended for not finishing in time.
#[derive(Debug)]
pub struct TimedOut(pub Duration);

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no answer in {} s", self.0.as_secs())
    }
}

impl std::error::Error for TimedOut {}

/// Runs c to its end, or to limit if given, when it's killed and the error is a TimedOut. Returns its exit status and
/// what it wrote to stdout and to stderr.
pub fn capture(mut c: Command, stdin: Option<&[u8]>, limit: Option<Duration>) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>)> {
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn()?;
    if let (Some(input), Some(mut w)) = (stdin, child.stdin.take()) {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut w, &input);
        });
    }
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Some(limit) = limit {
        let (pid, timed_out) = (child.id() as i32, timed_out.clone());
        std::thread::spawn(move || {
            if wait.recv_timeout(limit) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
                // SAFETY: a signal to the command this started, which hasn't been waited for yet.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        });
    }
    let mut err = child.stderr.take().unwrap();
    let said_err = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let mut out = Vec::new();
    let read = child.stdout.take().unwrap().read_to_end(&mut out);
    let status = child.wait();
    let _ = done.send(());
    let said_err = said_err.join().unwrap_or_default();
    if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(TimedOut(limit.unwrap_or_default()).into());
    }
    read?;
    Ok((status?, out, said_err))
}

fn run_for(limit: Option<Duration>, stdin: Option<&[u8]>, program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<String> {
    let program = program.as_ref();
    let mut c = cmd(program);
    c.args(args);
    let name = Path::new(program).file_name().unwrap_or(program).to_string_lossy().to_string();
    let spawned = capture(c, stdin, limit);
    // Both of what it says, in one: stderr into stdout's pipe isn't portable, so they're read apart and joined.
    let (status, mut out, said_err) = match spawned {
        Ok(done) => done,
        Err(err) if err.is::<TimedOut>() => return Err(err.context(format!("{name} {}", args[..args.len().min(2)].join(" ")))),
        Err(err) => return Err(anyhow!("{}: {err}", program.to_string_lossy())),
    };
    out.extend(said_err);
    let out = String::from_utf8_lossy(&out).to_string();
    if !status.success() {
        bail!("{name} {}: {status}: {}", args[..args.len().min(2)].join(" "), out.trim());
    }
    Ok(out)
}

/// A command run so that it outlives this service: on Linux in a scope of the user's own systemd, as a service's
/// restart stops everything in its own; on a Mac, in a session of its own.
pub fn outliving(program: &str, args: &[&str]) -> (String, Vec<String>) {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    if cfg!(target_os = "linux") && super::linux::found("systemd-run") {
        let mut all: Vec<String> = ["--user", "--scope", "--quiet", "--collect", "--", program].map(String::from).to_vec();
        all.extend(args);
        return ("systemd-run".into(), all);
    }
    (program.into(), args)
}

pub fn run_outliving(program: &str, args: &[&str]) -> Result<String> {
    let (p, a) = outliving(program, args);
    run(p, &a.iter().map(String::as_str).collect::<Vec<_>>())
}

/// A file given as a path here or a URL, downloaded to a temporary folder the caller removes (the second, if any).
pub fn fetch(app: &str) -> Result<(PathBuf, Option<PathBuf>)> {
    if !app.starts_with("http://") && !app.starts_with("https://") {
        if !Path::new(app).exists() {
            bail!("no file {app} on this machine");
        }
        return Ok((app.into(), None));
    }
    let resp = ureq::get(app).call().map_err(|err| anyhow!("downloading {app}: {err}"))?;
    let name = app.split(['?', '#']).next().unwrap_or_default().rsplit('/').next().unwrap_or_default().to_string();
    let dir = std::env::temp_dir().join(format!("agent-devices-app-{}", super::random_hex(6)));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(or(&name, "app"));
    let mut f = std::fs::File::create(&path)?;
    std::io::copy(&mut resp.into_body().into_reader(), &mut f)?;
    Ok((path, Some(dir)))
}

/// The resolvers this machine asks, not a stub of its own on 127.0.0.53, which a device can't reach.
pub fn upstream_dns() -> Vec<String> {
    let mut found = Vec::new();
    for file in ["/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"] {
        for line in std::fs::read_to_string(file).unwrap_or_default().lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() == 2
                && f[0] == "nameserver"
                && let Ok(std::net::IpAddr::V4(ip)) = f[1].parse::<std::net::IpAddr>()
                && !ip.is_loopback()
                && !found.contains(&f[1].to_string())
            {
                found.push(f[1].to_string());
            }
        }
    }
    found
}

/// Whether something here listens on the port.
pub fn listening(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(200)).is_ok()
}

/// Whether anything holds a connection to one of the ports here.
pub fn connected(ports: &[i64]) -> bool {
    ports.iter().filter(|p| **p > 0).any(|p| {
        cmd("ss").args(["-Htn", "state", "established", &format!("( sport = :{p} )")]).output().is_ok_and(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
    })
}

/// Whether something other than this service is connected to a device's screen at socket.
fn screen_watched(socket: &Path) -> bool {
    cmd("ss").args(["-Hx", "state", "established"]).output().is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains(&format!("{} ", socket.display())))
}

/// The processes listening on port on this machine.
fn listeners(port: u16) -> Vec<i32> {
    let out = if cfg!(target_os = "linux") {
        cmd("ss").args(["-Htlnp", &format!("( sport = :{port} )")]).output()
    } else {
        cmd("lsof").args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fp"]).output()
    };
    let out = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    // ss says users:(("one",pid=1,fd=9)), lsof -Fp a line p1 for each.
    let mut pids = Vec::new();
    for f in out.split([',', '\n', '(', ')']) {
        let pid = f.strip_prefix("pid=").or_else(|| if cfg!(target_os = "linux") { None } else { f.strip_prefix('p') });
        if let Some(pid) = pid.and_then(|p| p.parse::<i32>().ok()).filter(|p| *p > 0)
            && !pids.contains(&pid)
        {
            pids.push(pid);
        }
    }
    pids
}

/// A process's parent.
fn parent(pid: i32) -> i32 {
    cmd("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).output().ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()).unwrap_or(0)
}

/// Where adb's server listens, which any program reaching it can drive every phone here with.
pub const ADB_PORT: u16 = 5037;

/// The tools computer_use runs on a device of another kind.
pub fn vm_tools(k: &dyn Kind) -> String {
    [
        "screenshot: the screen now, at its own size; x and y in the other tools are its pixels",
        "click {x, y}, double_click {x, y}, right_click {x, y} (on Android, a long press)",
        "type_text {text}: types it where the focus is",
        "press_key {key}: a key, like enter, escape, backspace, tab, home, back (Android), or a combination like ctrl-c",
        "scroll {x, y, to_x, to_y}, swipe {x, y, to_x, to_y}: drags from one point to the other",
        "open_url {url}: opens it in the browser",
        "install_app {app}: a file on this machine or a URL (an .apk on Android, an .msi, .msix or .exe on Windows, an .app on an iPhone), or a winget id on Windows",
        "run_command {command}: runs it and gives its output (PowerShell on Windows, the shell on Android, a program in the simulator on an iPhone: it has no shell)",
        &format!("Input here: {}", k.limits()),
    ]
    .join("\n")
}

/// The memory a device takes, in GB.
pub fn gb_of(v: Option<&Vm>) -> u64 {
    v.map(|v| v.gb as u64).filter(|g| *g > 0).unwrap_or(4)
}

/// Whether this machine has gb of memory to spare, and 4 GB more for everything else it runs, as memory counts it
/// (less the swap in use): a machine short of memory kills what it must, a device or a build or an agent.
pub fn room_for(gb: u64) -> bool {
    super::pool::memory().0 >= (gb + 4) << 30
}

/// A lasting device's loan ends after this long without a call, a look or a connection to it; a copy's after
/// IDLE_BORROWED, as a Linux device's.
const IDLE_LASTING: Duration = Duration::from_secs(3600);

/// A borrowed copy left stopped this long without a call is given back: a call starts it again before then.
const IDLE_STOPPED_COPY: Duration = Duration::from_secs(120);

/// A device nobody has borrowed, called or watched for this long is paused, and after IDLE_STOP_VM shut down: each
/// holds cores and gigabytes while it runs.
const IDLE_PAUSE_VM: Duration = Duration::from_secs(15 * 60);
const IDLE_STOP_VM: Duration = Duration::from_secs(3600);

/// The display of a duration, as Go writes them.
fn go_duration(d: Duration) -> String {
    match d.as_secs() {
        s if s % 3600 == 0 => format!("{}h0m0s", s / 3600),
        s if s % 60 == 0 && s >= 3600 => format!("{}h{}m0s", s / 3600, s % 3600 / 60),
        s if s % 60 == 0 => format!("{}m0s", s / 60),
        s => format!("{s}s"),
    }
}

impl Devices {
    fn vm_dir(&self) -> PathBuf {
        self.dir.join("devices")
    }

    /// This machine's devices of other kinds.
    pub(super) fn vms(&self) -> Vec<Vm> {
        let _files = self.vm_files.lock().unwrap();
        let mut out: Vec<(String, Vm)> = std::fs::read_dir(self.vm_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| Some((e.file_name().to_string_lossy().to_string(), serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?)))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out.into_iter().map(|(_, v)| v).collect()
    }

    pub(super) fn vm_named(&self, name: &str) -> Option<Vm> {
        self.vms().into_iter().find(|v| v.name == name)
    }

    pub(super) fn save_vm(&self, v: &Vm) -> Result<()> {
        let _files = self.vm_files.lock().unwrap();
        std::fs::create_dir_all(self.vm_dir())?;
        let path = self.vm_dir().join(format!("{}.json", v.name));
        let part = path.with_extension("json.part");
        std::fs::write(&part, serde_json::to_vec_pretty(v)?)?;
        std::fs::rename(part, path)?;
        Ok(())
    }

    pub(super) fn forget_vm(&self, name: &str) {
        let _files = self.vm_files.lock().unwrap();
        let _ = std::fs::remove_file(self.vm_dir().join(format!("{name}.json")));
    }

    /// Whether a device here has the name already.
    fn taken(&self, name: &str) -> bool {
        self.vm_named(name).is_some() || self.list().is_ok_and(|all| super::linux::pick(&all, name).is_ok())
    }

    /// A device running and not paused, started or resumed if it has to be; it counts as a call to it.
    pub(super) fn awake(&self, v: &mut Vm) -> Result<()> {
        self.touch(&v.name);
        let k = v.kind();
        if !k.running(self, v) {
            if !room_for(gb_of(Some(v))) {
                bail!("{} is stopped, and this machine has no memory to spare to start it now", v.name);
            }
            v.paused = false;
            let _ = self.save_vm(v);
            k.start(self, v)?;
            self.want(&v.name, true);
            return Ok(());
        }
        if v.paused {
            k.resume(self, v)?;
            v.paused = false;
            return self.save_vm(v);
        }
        Ok(())
    }

    /// Makes a device of kind k here, named name: a copy of the lasting device from, or a new lasting one.
    pub(super) fn make_vm(&self, k: &dyn Kind, name: &str, from: Option<&Vm>) -> Result<Vm> {
        let why = k.can();
        if !why.is_empty() {
            bail!("{why}");
        }
        if !valid_name(name) {
            bail!("a device's name is lowercase letters, digits and dashes");
        }
        let _creating = self.creating.lock().unwrap();
        if self.taken(name) {
            bail!("there's already a device named {name}");
        }
        let mut v = Vm { name: name.into(), kind: k.name().into(), made: super::now(), of: from.map(|f| f.name.clone()).unwrap_or_default(), ..Default::default() };
        let (first, step) = k.port();
        if first > 0 {
            v.port = self.free_port(first, step);
        }
        k.make(self, &mut v, from)?;
        self.save_vm(&v)?;
        Ok(v)
    }

    /// Deletes a device and its files.
    pub(super) fn delete_vm(&self, v: &Vm) -> Result<()> {
        v.kind().remove(self, v)?;
        self.forget_vm(&v.name);
        self.unwant(&v.name);
        Ok(())
    }

    /// Lends the lasting device named, or of the kind, or a throwaway copy of one with copy (or when every lasting one
    /// is borrowed and this machine has room for a copy).
    fn borrow_vm(&self, k: &dyn Kind, name: &str, copy: bool, mut l: Loan) -> Result<Vm> {
        let why = k.can();
        if !why.is_empty() {
            bail!("{why}");
        }
        let take = self.take.lock().unwrap();
        let lent = self.loans();
        let lasting: Vec<Vm> = self.vms().into_iter().filter(|v| v.kind == k.name() && v.of.is_empty() && (name.is_empty() || v.name == name)).collect();
        if !name.is_empty() && lasting.is_empty() {
            bail!("there's no {} device named {name} here", k.title());
        }
        let mut got = None;
        let mut carries_on = false;
        if !copy {
            for v in &lasting {
                match lent.get(&v.name) {
                    None => {
                        got = Some(v.clone());
                        break;
                    }
                    Some(b) if b.state == LoanState::Lent && b.by == l.by && l.by != super::SOMEONE => {
                        got = Some(v.clone());
                        l.since = b.since;
                        carries_on = true;
                        break;
                    }
                    _ => {}
                }
            }
        }
        let mut got = match got {
            Some(v) => v,
            None => {
                if lasting.is_empty() && k.name() != "android" && k.name() != "iphone" {
                    bail!("there's no {} device here to borrow or copy: the person makes one with create", k.title());
                }
                let from = lasting.first();
                if !copy
                    && let Some(from) = from
                    && !room_for(gb_of(Some(from)))
                {
                    let b = lent.get(&from.name).cloned().unwrap_or_default();
                    bail!(
                        "{} is borrowed by {} for {}, since {}, and this machine has no room for a copy: ask them, or wait",
                        from.name,
                        b.by,
                        or(&b.r#for, "something"),
                        super::local(b.since, "%-I:%M%p")
                    );
                }
                if let Some(from) = from.filter(|f| !room_for(gb_of(Some(f)))) {
                    bail!("this machine has no memory to spare for a copy of {}", from.name);
                }
                self.make_vm(k, &super::pool::new_pool_name(), from)?
            }
        };
        if !carries_on {
            l.since = super::now();
            self.reserve(&got.name, l);
        } else {
            let r#for = l.r#for.clone();
            if !self.change(&got.name, |b| b.r#for = r#for) {
                bail!("{} is being given back: borrow again", got.name);
            }
        }
        drop(take);
        self.touch(&got.name);
        if let Err(err) = self.awake(&mut got) {
            if !got.of.is_empty() || got.name != name && got.name.starts_with(POOL_PREFIX) {
                let _ = self.delete_vm(&got);
            }
            self.end_loan(&got.name);
            return Err(err);
        }
        self.lend(&got.name);
        Ok(got)
    }

    /// Ends a loan: a copy, or a device made for the loan, is deleted; a lasting one stays as it is.
    pub(super) fn give_back_vm(&self, v: &Vm, why: &str) -> Result<()> {
        self.returning(&v.name)?;
        if v.throwaway() {
            if let Err(err) = self.delete_vm(v) {
                self.return_failed(&v.name);
                return Err(err);
            }
        } else if v.kind == "iphone" {
            // A booted simulator holds gigabytes of the Mac's memory.
            if v.kind().stop(self, v).is_ok() {
                self.want(&v.name, false);
            }
        }
        self.end_loan(&v.name);
        tracing::info!("{}: {why}", v.name);
        Ok(())
    }

    /// Gives back loans of devices left idle, and of devices that are gone, and pauses and then shuts down devices
    /// nobody is using.
    pub(super) fn tend_vms(&self) {
        self.chrome_off("Chrome isn't allowed here now");
        for (name, l) in self.loans() {
            let Some(v) = self.vm_named(&name) else { continue };
            if l.state == LoanState::Returning || l.state == LoanState::Preparing && l.since > self.started {
                continue;
            }
            let limit = if !v.throwaway() {
                IDLE_LASTING
            } else if v.state(self) == "stopped" {
                // A throwaway copy that stopped (its emulator killed for memory, or a reinstall mid-boot) holds its disk
                // and its loan for nothing: its borrower borrows another.
                IDLE_STOPPED_COPY
            } else {
                super::pool::IDLE_BORROWED
            };
            if self.idle(&name) < limit {
                continue;
            }
            if self.in_use(&v) {
                self.touch(&name);
                continue;
            }
            if let Err(err) = self.give_back_vm(&v, &format!("idle for {} (borrowed by {} for {})", go_duration(limit), l.by, l.r#for)) {
                tracing::warn!("giving back {name}: {err:#}");
            }
        }
        self.prune_bases();
        self.rest_vms();
    }

    /// This machine's devices of other kinds, as list shows them.
    pub(super) fn list_vms(&self, me: &str, lent: &HashMap<String, Loan>) -> Vec<super::everywhere::Listed> {
        self.vms()
            .into_iter()
            .map(|v| super::everywhere::Listed {
                name: v.name.clone(),
                on: me.into(),
                kind: v.kind.clone(),
                state: v.state(self).into(),
                of: v.of.clone(),
                cpus: v.cpus as f64,
                memory_limit: (v.gb as i64) << 30,
                borrowed: lent.get(&v.name).cloned(),
                ..Default::default()
            })
            .collect()
    }

    /// Runs one of `vm_tools` on a running device.
    fn use_vm(&self, v: &mut Vm, tool: &str, args: &Input) -> Result<Value> {
        let k = v.kind();
        if !k.running(self, v) {
            bail!("{} is stopped: start it first", v.name);
        }
        self.awake(v)?;
        let t = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let f = |key: &str| number(args, key).unwrap_or(0.0);
        let mut a = Action { x: f("x"), y: f("y"), to_x: f("to_x"), to_y: f("to_y"), text: t("text"), key: t("key"), ..Default::default() };
        a.r#do = match tool {
            "screenshot" => return self.screen_vm(v, 0),
            "click" | "double_click" | "right_click" | "scroll" | "swipe" => tool.into(),
            "type_text" => "type".into(),
            "press_key" => "key".into(),
            "open_url" => {
                if t("url").is_empty() {
                    bail!("open which url?");
                }
                k.open(self, v, &t("url"))?;
                return Ok(json!("Opened."));
            }
            "install_app" => return Ok(json!(k.install(self, v, &or(&t("app"), &t("file")))?)),
            "run_command" => return Ok(json!(k.run(self, v, &t("command"))?)),
            _ => bail!("no tool {tool:?} on {}: there's\n{}", v.name, vm_tools(k)),
        };
        k.act(self, v, &a)?;
        Ok(json!("Done: take a screenshot to see what it did."))
    }

    /// A JPEG of the device's screen, width wide (0: its own size, at most 1600).
    pub(super) fn screen_vm(&self, v: &Vm, width: u32) -> Result<Value> {
        let img = v.kind().screen(self, v)?;
        let (b, w, h) = super::picture::jpeg_of(&img, width)?;
        Ok(super::picture_answer(&b, format!("{} now, {w}x{h}; its own screen is {}x{}, the pixels x and y are in.", v.name, img.width(), img.height())))
    }

    /// Whose a port is, of what only this machine's own programs use on its 127.0.0.1: its node's (its local API, and
    /// its apps' runtime and their requests out), adb's server, which drives every phone here, Agent Devices' viewer,
    /// and each device's controls, screen and shell. "" if it's none of those.
    fn local_only(&self, port: u16) -> String {
        let node_port = self.host.port();
        if node_port == port {
            return "this machine's node's".into();
        }
        let node = listeners(node_port);
        if !node.is_empty() && listeners(port).iter().any(|pid| node.contains(pid) || node.contains(&parent(*pid))) {
            return "this machine's node's".into();
        }
        if port == ADB_PORT {
            return "adb's".into();
        }
        if port == super::viewer::VIEWER_PORT {
            return "Agent Devices' viewer's".into();
        }
        for v in self.vms() {
            // Android: its console, and adb's port after it. Windows: its shell (SSH), and its screen (VNC).
            if port == v.port || (v.kind == "android" && port == v.port + 1) || (v.display > 0 && port == 5900 + v.display) {
                return format!("{}'s", v.name);
            }
        }
        for d in self.list().unwrap_or_default() {
            if d.port == port {
                return format!("{}'s", d.name);
            }
        }
        String::new()
    }

    /// The first port from first, every step, no device here uses and nothing listens on.
    fn free_port(&self, first: u16, step: u16) -> u16 {
        let used: Vec<u16> = self.vms().iter().map(|v| v.port).collect();
        let mut p = first;
        while used.contains(&p) || listening(p) {
            p += step;
        }
        p
    }

    /// The kinds this machine runs, each with its title and what input it takes.
    pub(super) fn kinds_here(&self) -> Vec<Value> {
        let mut out = Vec::new();
        let lent = self.loans();
        if self.rt.is_some() {
            let all = self.list().unwrap_or_default();
            out.push(json!({ "kind": LINUX, "title": "Linux with a browser", "copies": true, "room": !super::pool::spares(&all, &lent).is_empty() || super::pool::room_for_one() }));
        }
        for k in RUNTIMES {
            if k.name() == super::chrome::NAME {
                // Listed even where it can't run, with why: its owner may allow it.
                let why = self.chrome_why_not();
                if !why.is_empty() {
                    out.push(json!({ "kind": k.name(), "title": k.title(), "unavailable": why, "room": false }));
                } else {
                    out.push(json!({ "kind": k.name(), "title": k.title(), "input": k.limits(), "room": room_for(super::chrome::GB) }));
                }
                continue;
            }
            if k.can().is_empty() {
                let unborrowed = self.vms().iter().filter(|v| v.kind == k.name() && v.of.is_empty() && !lent.contains_key(&v.name)).count();
                out.push(json!({ "kind": k.name(), "title": k.title(), "input": k.limits(), "room": room_for(gb_of(None)), "unborrowed": unborrowed }));
            }
        }
        out
    }

    /// Runs the functions that pick a kind, or act on a device of a kind other than linux; none for the rest, which
    /// the Linux devices' code runs.
    pub(super) fn run_kinds(&self, caller: &Caller, function: &str, name: &str, here: bool, input: &mut Input) -> Option<Result<Value>> {
        let mut kind_name = text(input, "kind");
        if kind_name == LINUX {
            kind_name.clear();
        }
        let v = if name.is_empty() { None } else { self.vm_named(name) };
        match function {
            "kinds" => {
                let me = self.me();
                let mine = json!({ "device": me, "kinds": self.kinds_here(), "free_gb": super::pool::memory().0 >> 30 });
                if here {
                    return Some(Ok(mine));
                }
                let (got, failed) = self.everywhere("kinds", None);
                let mut all = vec![mine];
                let mut names: Vec<&String> = got.keys().collect();
                names.sort();
                all.extend(names.into_iter().map(|s| got[s].clone()));
                let mut out = json!({ "devices": all });
                if !failed.is_empty() {
                    out["unreachable"] = json!(failed);
                }
                return Some(Ok(out));
            }
            "adopt" => return Some(self.adopt(name, &kind_name, input)),
            "create" | "borrow" => {
                if let Some(v) = &v {
                    kind_name = v.kind.clone();
                }
                if kind_name.is_empty() {
                    return None;
                }
                return Some(self.create_or_borrow(caller, function, name, here, &kind_name, v.is_some(), input));
            }
            _ => {}
        }
        let Some(mut v) = v else {
            if ["open", "install", "run"].contains(&function) {
                return Some(self.run_linux(function, name, input));
            }
            return None;
        };
        let k = v.kind();
        if !k.can().is_empty() {
            return Some(Err(anyhow!("{} is a {} device, which this machine can't run now", v.name, v.kind)));
        }
        if v.kind == super::chrome::NAME && super::BROWSING.contains(&function) {
            return Some(self.browse(&mut v, function, input));
        }
        Some((|| -> Result<Value> {
            match function {
                "start" => {
                    self.awake(&mut v)?;
                    Ok(json!("Started."))
                }
                "stop" => {
                    v.paused = false;
                    let _ = self.save_vm(&v);
                    k.stop(self, &v)?;
                    self.want(&v.name, false);
                    Ok(json!(format!("Stopped {}. Its files stay; it stays stopped until started.", v.name)))
                }
                "delete" => {
                    self.delete_vm(&v)?;
                    self.end_loan(&v.name);
                    Ok(json!(format!("Deleted {} and everything in it.", v.name)))
                }
                "give_back" => {
                    if !self.loans().contains_key(&v.name) {
                        bail!("{} isn't borrowed", v.name);
                    }
                    self.give_back_vm(&v, &format!("given back by {}", caller.name()))?;
                    Ok(json!(format!("Gave back {}.", v.name)))
                }
                "view" => {
                    self.awake(&mut v)?;
                    let m = minutes(input, 60, 1440);
                    let url = super::viewer::vm_view_url(&v, m)?;
                    Ok(json!(format!("{url}\nThe link works for {m} minutes, on the person's own devices.")))
                }
                "screen" => self.screen_now(&v.name, super::width(input)),
                "tools" => Ok(json!(vm_tools(k))),
                "use" => {
                    let tool = text(input, "tool");
                    let args = input.get("args").and_then(Value::as_object).cloned().unwrap_or_default();
                    self.use_vm(&mut v, &tool, &args)
                }
                "open" => self.use_vm(&mut v, "open_url", input),
                "install" => self.use_vm(&mut v, "install_app", input),
                "run" => self.use_vm(&mut v, "run_command", input),
                "reach" => {
                    let port = number(input, "port").unwrap_or(0.0);
                    if !(1.0..=65535.0).contains(&port) {
                        bail!("reach which port? a number from 1 to 65535");
                    }
                    let port = port as u16;
                    let mine = self.local_only(port);
                    if !mine.is_empty() {
                        bail!("no device reaches port {port}: it's {mine}, for this machine's own programs only");
                    }
                    if !k.running(self, &v) {
                        bail!("{} is stopped: start it first", v.name);
                    }
                    Ok(json!(k.reach(self, &v, port)?))
                }
                _ => bail!("{} is a {} device: {function} works on Linux devices only, for now", v.name, k.title()),
            }
        })())
    }

    #[allow(clippy::too_many_arguments)]
    fn create_or_borrow(&self, caller: &Caller, function: &str, name: &str, here: bool, kind_name: &str, named: bool, input: &Input) -> Result<Value> {
        let k = kind_named(kind_name)?;
        if k.name() == super::chrome::NAME && function == "create" {
            bail!("chrome devices are only borrowed, each by one agent: borrow one with kind chrome");
        }
        if !named && !here && let Some(r) = self.first(function, k.name(), name, input) {
            return r;
        }
        if k.name() == super::chrome::NAME {
            return self.borrow_chrome(caller, input);
        }
        let why = k.can();
        if !why.is_empty() {
            bail!("none of your devices runs {} agent devices (here: {why})", k.title());
        }
        if function == "create" {
            if name.is_empty() {
                bail!("name it: lowercase letters, digits and dashes");
            }
            let mut v = self.make_vm(k, name, None)?;
            self.awake(&mut v).map_err(|err| anyhow!("made {}, but it didn't start: {err:#}", v.name))?;
            return Ok(json!(format!("Made {}, a lasting {} device, and started it.", v.name, k.title())));
        }
        let why = text(input, "for");
        if why.is_empty() {
            bail!("say what it's for, in a few words: the person sees it");
        }
        let got = self.borrow_vm(k, name, flag(input, "copy"), Loan { by: or(&text(input, "by"), &caller.name()), r#for: why, ..Default::default() })?;
        let me = self.me();
        let what = if !got.of.is_empty() {
            format!("{}, a throwaway copy of {}, deleted with everything in it when you give it back", got.name, got.of)
        } else if got.name.starts_with(POOL_PREFIX) {
            format!("{}, a throwaway {} device of your own, deleted with everything in it when you give it back", got.name, k.title())
        } else {
            format!("{}, the person's lasting {} device: leave it as you found it", got.name, k.title())
        };
        let msg = format!(
            "Borrowed {what}. Pass device {:?} to computer_screen and computer_use (computer_tools says what it takes: {}). Give it back with give_back when done.",
            got.name,
            k.limits()
        );
        Ok(json!({ "device": got.name, "on": me, "kind": got.kind, "copy": !got.of.is_empty(), "text": msg }))
    }

    /// Makes something already on this machine a lasting device: a Windows VM's folder (its disk.qcow2, OVMF_VARS.fd,
    /// tpm and monitor.sock, made as windows.rs makes them), an Android Virtual Device, or a simulator.
    fn adopt(&self, name: &str, kind_name: &str, input: &Input) -> Result<Value> {
        let num = |k: &str| number(input, k).unwrap_or(0.0) as u16;
        let k = kind_named(kind_name)?;
        let why = k.can();
        if !why.is_empty() {
            bail!("{why}");
        }
        if !valid_name(name) {
            bail!("a device's name is lowercase letters, digits and dashes");
        }
        if self.taken(name) {
            bail!("there's already a device named {name}");
        }
        let mut v = Vm {
            name: name.into(),
            kind: k.name().into(),
            made: super::now(),
            cpus: num("cpus"),
            gb: num("gb"),
            port: num("port"),
            display: num("display"),
            id: text(input, "id"),
            dir: text(input, "dir"),
            ..Default::default()
        };
        if v.gb == 0 && k.name() == "iphone" {
            v.gb = 2;
        }
        k.adopt(self, &mut v)?;
        self.save_vm(&v)?;
        Ok(json!(format!("{} is a lasting {} device now, {}.", v.name, k.title(), v.state(self))))
    }

    /// Runs open, install and run on a Linux device.
    fn run_linux(&self, function: &str, name: &str, input: &Input) -> Result<Value> {
        let d = self.running(name)?;
        let script = match function {
            "open" => {
                let url = text(input, "url");
                if url.is_empty() {
                    bail!("open which url?");
                }
                format!("runuser -u cua -- env DISPLAY=:1 HOME={HOME} setsid google-chrome --no-first-run --no-default-browser-check --password-store=basic {} >/dev/null 2>&1 </dev/null &", quote(&url))
            }
            "install" => {
                let app = or(&text(input, "app"), &text(input, "file"));
                if app.is_empty() {
                    bail!("install what? a package's name, or a .deb's URL");
                } else if app.starts_with("http://") || app.starts_with("https://") {
                    format!("set -e; curl -fsSLo /tmp/app.deb {} && apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq /tmp/app.deb; rm -f /tmp/app.deb", quote(&app))
                } else {
                    format!("set -e; apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {}", quote(&app))
                }
            }
            _ => text(input, "command"),
        };
        if script.is_empty() {
            bail!("run what? pass command");
        }
        let mut out = Vec::new();
        if let Err(err) = self.exec(&d, Some(&mut script.as_bytes()), Some(&mut out), &["sh", "-s"]) {
            bail!("{err:#}\n{}", String::from_utf8_lossy(&out));
        }
        if function == "open" {
            return Ok(json!("Opened."));
        }
        Ok(json!(String::from_utf8_lossy(&out)))
    }

    /// Pauses the running devices nobody has used for IDLE_PAUSE_VM, and shuts down those idle for IDLE_STOP_VM.
    fn rest_vms(&self) {
        let lent = self.loans();
        for mut v in self.vms() {
            let k = v.kind();
            if lent.contains_key(&v.name) || !k.can().is_empty() || !k.running(self, &v) {
                continue;
            }
            if self.in_use(&v) {
                self.touch(&v.name);
                continue;
            }
            let idle = self.idle(&v.name);
            if idle >= IDLE_STOP_VM {
                tracing::info!("stopping {}, idle for an hour", v.name);
                match k.stop(self, &v) {
                    Err(err) => tracing::warn!("stopping {}: {err:#}", v.name),
                    Ok(()) => self.want(&v.name, false),
                }
                v.paused = false;
                let _ = self.save_vm(&v);
            } else if idle >= IDLE_PAUSE_VM && !v.paused {
                tracing::info!("pausing {}, idle for 15 minutes", v.name);
                if let Err(err) = k.pause(self, &v) {
                    tracing::warn!("pausing {}: {err:#}", v.name);
                    continue;
                }
                v.paused = k.running(self, &v);
                if !v.paused {
                    self.want(&v.name, false); // pausing stopped it, as an iPhone's does
                }
                let _ = self.save_vm(&v);
            }
        }
    }

    /// Whether something uses a device without calling this service: an SSH or VNC session into Windows, or more than
    /// adb's own chatter (512 KB in a look) to a phone.
    fn in_use(&self, v: &Vm) -> bool {
        match v.kind.as_str() {
            "windows" => connected(&[v.port as i64]) || screen_watched(&v.file("vnc.sock")),
            "android" => {
                let Ok(out) = cmd("ss").args(["-tinH", "state", "established", &format!("( dport = :{} )", v.port + 1)]).output() else { return false };
                if !out.status.success() {
                    return false;
                }
                let n: i64 = String::from_utf8_lossy(&out.stdout).split_whitespace().filter_map(|f| f.strip_prefix("bytes_acked:")).filter_map(|b| b.parse::<i64>().ok()).sum();
                let was = self.traffic.lock().unwrap().insert(v.name.clone(), n);
                was.is_some_and(|was| n - was > 512 << 10)
            }
            _ => false,
        }
    }
}
