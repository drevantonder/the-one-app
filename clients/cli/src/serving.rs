//! This device as one of its network's servers. Three state machines, each the one place its state changes:
//!
//! - `server_membership` (`ServerMembership`), whether it's a server: inactive, then joining (`one serve on`:
//!   registering, downloading, installing, starting) until it answers by its key, then active, then leaving
//!   (`one serve off`: draining, deregistering, uninstalling), then inactive again. Either that fails goes back to
//!   where it started, undoing what it did, and says why, as does either that nothing is doing any more, as one the
//!   machine went off during, whichever of this program's starts on the device comes first (`settle`).
//! - `server_daemon_status`, the program the system keeps running (`one serve run`): starting until the server answers
//!   by its key, running, stopping, stopped. Restarts and their back-off are the system's: a systemd user unit on
//!   Linux, a LaunchAgent on a Mac, a scheduled task on Windows (`system`). This program only ends, non-zero where it
//!   failed.
//! - `server_update_status` (`ServerUpdateStatus`), while it's active: up to date, downloading the latest server
//!   build and `one` together and checking the build starts (`prepare`), checked, verifying the pair once it's told
//!   to go (`go`), which is starting both in the server's place, as every update restarts it, and up to date again,
//!   kept or gone back. Its network's updater (`core/edge/updater/updater.ts`) decides when, one server at a time.
//!   A build is started only by the `one` the record pairs it with.
//!
//! ```text
//! server_membership:
//!   inactive ─serve on─► joining (registering → downloading → installing → starting) ─answers─► active
//!                          └─fails, or cut short─► inactive, off the list, nothing left (undo_joining)
//!   active ─serve off─► leaving (draining → deregistering → uninstalling) ─► inactive, nothing left
//!                          └─fails, or cut short─► active, back on the list, run by the system (undo_leaving)
//!   cut short: settled by this program's first start on the device, unit or not (`settle`): `serve run`,
//!   `serve on`, `serve off` (which then leaves again), `update`, the app; joining at starting is `serve run`'s
//!
//! server_update_status:
//!   up to date ─prepare─► downloading (the build, and `one` beside this one, as the server lists them) ─► checking, in
//!     one VM of the build
//!     ─► checked ─go─► verifying: starting (the new `one` put in place) ─said at the edge─► up to date on the new pair
//!                                                 └─doesn't─► up to date, went back
//!     nothing newer, or a download or check that failed ─► up to date, staying on it, saying why
//!   cut short: downloading ─► up to date · checked stays checked · starting ─► started again
//!   every start of `serve run`: the `one` the record pairs with the build is put in place and started first
//!   said at the edge: the build started says it's running at the network's edge, by its key. Nothing else is asked,
//!     so no other server, or an app it runs, has a say: every server can take a build at once
//! ```
//!
//! What it does is reported for Telemetry as it happens (`Reports`), kept on the device until a server is told, so a
//! server that won't start is reported by its machine all the same: each start, with how long it took and why the
//! run before ended (`one.server.start`, an error where it didn't start); each stop, an error where it wasn't asked
//! (`one.server.stop`); ending unasked `LOOP_ENDS` times within `LOOP` (`one.server.crash_loop`); and each update,
//! how it was taken, how long it took, and whether it was kept, went back or stayed, and why (`one.server.update`).
//!
//! A server that missed a release, as one asleep or away during it, is caught up by its network's update once it's
//! back, which it hears of as the server says it's running, or can reach the network again, at the network's edge.
//!
//! Every start is the one way: a build is started and has `START` to say it's running at the network's edge. What
//! each machine is at is one small record, `state.json` in the client's home's `server/`, which every program changes
//! in one step under one lock (`Here::change`), beside `builds/<build>`, each build kept, `runtimes/<build>`, each
//! runtime kept, `programs/<program>/<build>`, each `one` a pair names, kept by which it is (`one` or `one-cli`), so one
//! of the other put in place by a person, as installing the app over `one-cli` does, runs the server from then on
//! rather than being put back, and `home`,
//! the server's own folder, its key among it. A build is put together from its runtime, as hard links, and its code
//! (`Here::assemble`), so it's a whole Erlang release that any start starts. The server is told on its input what
//! changes of it (`OneCore.Release`), and says it to the network, where it's ready once it's active and running. What
//! the two say to each other is `client::serve`'s.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;

use client::serve::{
    Said, ServerDaemonStatus, ServerMembership, ServerUpdateStage, ServerUpdateStatus, Status, Told,
};
use client::{BUILD, Client, target};

/// Which build of `one` an update takes the latest of: the app, with its window, or `one` built without it.
const PROGRAM: &str = if cfg!(feature = "window") { "one" } else { "one-cli" };
/// The public half of the key the platform signs each build it publishes with, BUILD_SIGNING_KEY in `tooling/release`
/// (in 1Password, "The One App build signing key").
const KEY: &str = "3fd5f3516020dafc3690044ac56dceef578b1f162b72ec4687af2d764192d2f2";
/// Where the core's latest builds are, for a new network's first server, which has no network to download from yet:
/// a list for each kind of machine, `<target>.json`, as the network's `download` gives it, which `tooling/release`
/// keeps as the network's latest.
const RELEASES: &str = "https://github.com/ptc-au/the-one-app/releases/download/builds";
/// How long every start of a build has to say it's running at the network's edge.
const START: Duration = Duration::from_secs(180);
/// How long a server is given to stop: longer than it takes to answer what it took (`OneCore.Application`).
const STOP: Duration = Duration::from_secs(20);
/// How many of the last lines a build said are kept to say why it didn't start.
const LAST: usize = 5;
/// A server that ends without being asked `LOOP_ENDS` times within `LOOP` is in a crash loop.
const LOOP: Duration = Duration::from_secs(600);
const LOOP_ENDS: usize = 3;
/// How long what's reported has to reach a server as this program ends, before it's kept for next time.
const TELLING: Duration = Duration::from_secs(5);

/// Where joining or leaving is: each has the one way out on failure, back to where it began.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerMembershipStage {
    Registering,
    Downloading,
    Installing,
    Starting,
    Draining,
    Deregistering,
    Uninstalling,
}

/// A server's build and the `one` that runs it, which an update takes, keeps and goes back from together.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Pair {
    build: String,
    program: String,
}

/// What this device is as a server, written in one step (`Here::write`).
#[derive(Serialize, Deserialize, Default, Debug)]
struct Record {
    server_membership: ServerMembership,
    server_membership_stage: Option<ServerMembershipStage>,
    server_update_status: ServerUpdateStatus,
    server_update_stage: Option<ServerUpdateStage>,
    current: Option<Pair>,
    previous: Option<Pair>,
    next: Option<Pair>,
    /// Why the last update didn't take, which the server says until one does.
    went_back: Option<String>,
    /// Why the server's last run ended, as this program saw it, which its next start reports.
    ended: Option<String>,
    /// When it ended without being asked within the last `LOOP`, in milliseconds since 1970.
    #[serde(default)]
    unasked: Vec<u64>,
    /// When the update being taken by restarting was told to go, in milliseconds since 1970.
    update_began: Option<u64>,
    /// Whom the server is for (`ONE_SUBJECT`), as it was joined and says as it starts, so `one serve on` registers it
    /// for them again whoever runs it.
    #[serde(default)]
    subject: Option<String>,
}

impl Record {
    /// What's told the server of itself, as its daemon is `daemon`: on its input, and as any build of it is started.
    fn told(&self, daemon: ServerDaemonStatus) -> Told {
        Told::Status(Status {
            server_membership: self.server_membership,
            server_daemon_status: daemon,
            server_update_status: self.server_update_status,
            server_update_stage: self.server_update_stage,
            next_build: self.next.as_ref().map(|next| next.build.clone()),
            program_name: Some(name()),
            program: self.pair().map(|pair| pair.program),
            next_program: self.next.as_ref().map(|next| next.program.clone()),
            went_back: self.went_back.clone(),
        })
    }

    /// The pair a start of the server runs: the one being verified, or the one it's on.
    fn pair(&self) -> Option<Pair> {
        if self.server_update_status == ServerUpdateStatus::Verifying {
            self.next.clone()
        } else {
            self.current.clone()
        }
    }

    /// Goes back from the pair being verified, which didn't take for `why`, to the one it's on, saying so until an
    /// update takes. The `one` paired with it is put in place as it's started (`run`).
    fn go_back(&mut self, why: impl std::fmt::Display) {
        let build = |pair: &Option<Pair>| pair.as_ref().map(|pair| pair.build.clone()).unwrap_or_default();
        self.went(format!("Went back to {} from {}: {why}", build(&self.current), build(&self.next)));
        self.next = None;
        self.server_update_status = ServerUpdateStatus::UpToDate;
        self.server_update_stage = None;
    }

    /// Says why the last update didn't take, which the server says until one does.
    fn went(&mut self, why: String) {
        println!("{why}");
        self.went_back = Some(why);
    }

    /// Keeps the pair being verified, `next`, which took: the server is on it, up to date.
    fn took(&mut self, next: Pair) {
        self.previous = self.current.replace(next);
        self.next = None;
        self.went_back = None;
        self.server_update_status = ServerUpdateStatus::UpToDate;
        self.server_update_stage = None;
    }

    /// The server ended without being asked to, for `why`, which its next start reports. Gives how many times it has
    /// within `LOOP`.
    fn ended_unasked(&mut self, why: &str) -> usize {
        let now = now();
        self.unasked.retain(|at| now.saturating_sub(*at) < LOOP.as_millis() as u64);
        self.unasked.push(now);
        self.ended = Some(why.to_owned());
        self.unasked.len()
    }
}

/// What this program reports of its server for Telemetry (`Client::report`), each record saying which server.
struct Reports {
    client: Arc<Client>,
    server: String,
}

impl Reports {
    fn report(&self, mut record: Value) {
        record["attributes"]["one.server"] = json!(self.server);
        self.client.report(record);
    }

    /// The server ended without being asked to, `ends` times within `LOOP`: a stop that failed, and a crash loop once
    /// it's `LOOP_ENDS`.
    fn ended_unasked(&self, record: Value, ends: usize, why: &str) {
        self.report(record);
        if ends >= LOOP_ENDS {
            let minutes = LOOP.as_secs() / 60;
            self.report(json!({"name": "one.server.crash_loop", "error": format!("It ended {ends} times in {minutes} minutes without being asked. The last: {why}")}));
        }
    }

    /// An update, as it ended: from which build to which, how long it took, and `went_back` or `stayed` with why where
    /// it wasn't kept.
    fn update(&self, from: &str, to: &str, took: Option<Duration>, error: Option<String>, outcome: &str) {
        let mut record = json!({"name": "one.server.update", "attributes": {
            "one.update.from": from, "one.update.to": to, "one.update.outcome": outcome,
        }});
        if let Some(took) = took {
            record["took"] = json!(took.as_secs_f64() * 1000.0);
        }
        if let Some(error) = error {
            record["error"] = json!(error);
        }
        self.report(record);
    }

    /// Tells a server now what's reported: asking any server anything has the client tell it what's kept, and what
    /// no server answers stays kept on the device for next time. Ending, it waits for that, within `TELLING`.
    async fn tell(&self, ending: bool) {
        let _ = timeout(TELLING, self.client.request("ping", json!({}))).await;
        if ending {
            let _ = timeout(TELLING, self.client.close()).await;
        }
    }
}

/// Now, in milliseconds since 1970.
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// How long since `at`, in milliseconds since 1970.
fn since(at: u64) -> Duration {
    Duration::from_millis(now().saturating_sub(at))
}

/// A line the server said, and the message it is, if it's one: only a line on its output is.
struct Heard {
    line: String,
    said: Option<Said>,
}

/// This device's server, in the client's home.
struct Here {
    folder: PathBuf,
}

mod builds;
mod daemon;
mod here;
mod membership;
mod update;

pub use builds::*;
pub use daemon::*;
pub use membership::*;
pub use update::*;

/// What's the server's own of how the system keeps it running (`client::system`): on Linux, where systemd waits for it to
/// say it's running (`Type=notify`) and keeps what it says in its journal.
#[cfg(target_os = "linux")]
mod system {
    use super::*;

    /// Has systemd start it now, or again where it's running, with what it's run with now. That returns once the
    /// server answers by its key (`ready`), or failed to start.
    pub async fn start(_here: &Here, server: &client::system::Program) -> Result<()> {
        client::system::restart(server)
    }

    /// What the server says is in systemd's journal, which keeps it within its own bounds.
    pub fn bound(_log: &Path) -> Result<()> {
        Ok(())
    }

    /// Where to read what the server says: its unit's journal.
    pub fn log(_here: &Here, key: &str) -> String {
        format!("`journalctl --user -u {}`", client::system::name(SERVER, key))
    }

    /// Tells systemd the server is running, as `Type=notify` has it wait for.
    pub fn ready(_key: &str) -> Result<()> {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::{SocketAddr, UnixDatagram};
        let Some(socket) = env::var_os("NOTIFY_SOCKET") else { return Ok(()) };
        let socket = socket.to_string_lossy();
        let address = match socket.strip_prefix('@') {
            Some(name) => SocketAddr::from_abstract_name(name)?,
            None => SocketAddr::from_pathname(&*socket)?,
        };
        UnixDatagram::unbound()?.send_to_addr(b"READY=1", &address)?;
        Ok(())
    }
}

/// What's the server's own of how the system keeps it running (`client::system`) on a Mac, where launchd has no limit
/// on how long a start takes, which is why every start has `START` here.
#[cfg(target_os = "macos")]
mod system {
    use std::os::unix::net::UnixDatagram;

    use super::*;

    /// Has launchd load the agent, which starts it, again where it's loaded. That returns once the server says it's
    /// running on a socket in its folder (`ready`), as its environment says.
    pub async fn start(here: &Here, server: &client::system::Program) -> Result<()> {
        let ready = here.folder.join("ready");
        let _ = fs::remove_file(&ready);
        let socket = UnixDatagram::bind(&ready)?;
        socket.set_read_timeout(Some(2 * START))?;
        client::system::restart(server)?;
        let running = socket.recv(&mut [0; 16]);
        fs::remove_file(&ready)?;
        running.with_context(|| {
            format!("The server didn't start: its log is {}", here.folder.join("server.log").display())
        })?;
        Ok(())
    }

    /// Where to read what the server says: `server.log` in its folder.
    pub fn log(here: &Here, _key: &str) -> String {
        here.folder.join("server.log").display().to_string()
    }

    /// Keeps `server.log` under `LOG`, as launchd keeps no bounds of its own: past it, the log is `server.log.1`, in
    /// place of the one before, and this program's output and errors go to a new one, as a daemon opens its log
    /// again. Only while its output is that file, as launchd started it.
    pub fn bound(log: &Path) -> Result<()> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;

        const LOG: u64 = 10 << 20;
        let Ok(metadata) = fs::metadata(log) else { return Ok(()) };
        let mut out: libc::stat = unsafe { std::mem::zeroed() };
        let writing = unsafe { libc::fstat(1, &mut out) } == 0
            && (out.st_dev as u64, out.st_ino as u64) == (metadata.dev(), metadata.ino());
        if metadata.len() < LOG || !writing {
            return Ok(());
        }
        fs::rename(log, log.with_extension("log.1"))?;
        let new = fs::OpenOptions::new().create(true).append(true).open(log)?;
        for written in [1, 2] {
            ensure!(unsafe { libc::dup2(new.as_raw_fd(), written) } >= 0, "couldn't write a new {}", log.display());
        }
        Ok(())
    }

    /// Tells `start` the server is running. Nothing hears it when launchd started it by itself, as at a login.
    pub fn ready(_key: &str) -> Result<()> {
        let Some(socket) = env::var_os("NOTIFY_SOCKET") else { return Ok(()) };
        let _ = UnixDatagram::unbound()?.send_to(b"READY=1", socket);
        Ok(())
    }
}

/// What's the server's own of how the system keeps it running (`client::system`) on Windows, as its person's scheduled
/// task. `one` asks for no console window as it's started there (`clients/app/build.rs`). What it says is `server.log`
/// in the server's folder.
#[cfg(windows)]
mod system {
    use std::os::windows::io::AsRawHandle;
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    use super::*;

    /// Where `start` hears that the server is running. A named pipe, as a task runs in its person's session, which
    /// isn't always the one `serve on` is run in.
    fn ready_pipe(key: &str) -> String {
        format!(r"\\.\pipe\one-server-{}-ready", &key[..12])
    }

    /// Has Task Scheduler start it now. That returns once the server answers by its key (`ready`).
    pub async fn start(here: &Here, server: &client::system::Program) -> Result<()> {
        let ready = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .create(ready_pipe(&server.key))?;
        client::system::restart(server)?;
        let log = here.folder.join("server.log");
        timeout(2 * START, ready.connect())
            .await
            .ok()
            .and_then(Result::ok)
            .with_context(|| format!("The server didn't start: its log is {}", log.display()))
    }

    /// Has Task Scheduler start the task again as soon as this program has ended, as one ending to run a newer build
    /// does. A task that's running isn't started (`IgnoreNew`), and a program that ends is no failure to Task
    /// Scheduler, so otherwise only its trigger starts it, within a minute. So a PowerShell that outlives this waits
    /// for it to end and starts the task, as `Start-ScheduledTask` does; where that doesn't, the trigger still does.
    pub fn again(key: &str) {
        let script = format!(
            "Wait-Process -Id {} -ErrorAction SilentlyContinue; Start-ScheduledTask -TaskName '{}'",
            std::process::id(),
            client::system::name(SERVER, key)
        );
        let started = command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(error) = started {
            eprintln!("Task Scheduler starts it within a minute, as it couldn't be had to at once: {error}");
        }
    }

    /// Tells `start` the server is running, by opening its pipe, which is all `start` waits for. Only to read it,
    /// which a pipe lets everyone do: one made in an ssh session lets nobody but administrators write to it, so
    /// writing to it from the task, its person's own, was refused. Nothing hears it when Task Scheduler started it
    /// by itself.
    pub fn ready(key: &str) -> Result<()> {
        let _ = fs::File::open(ready_pipe(key));
        Ok(())
    }

    /// The log this program writes, which its output and errors are, as a task's go nowhere.
    static LOG: Mutex<Option<fs::File>> = Mutex::new(None);

    /// Where to read what the server says: `server.log` in its folder.
    pub fn log(here: &Here, _key: &str) -> String {
        here.folder.join("server.log").display().to_string()
    }

    /// Has this program's output and errors go to `server.log`, kept under 10 MB: past it, the log is
    /// `server.log.1`, in place of the one before, and a new one is begun.
    pub fn bound(log: &Path) -> Result<()> {
        let mut writing = LOG.lock().unwrap();
        let full = fs::metadata(log).is_ok_and(|metadata| metadata.len() >= 10 << 20);
        if writing.is_some() && !full {
            return Ok(());
        }
        if full {
            fs::rename(log, log.with_extension("log.1"))?;
        }
        let new = fs::OpenOptions::new().create(true).append(true).open(log)?;
        for written in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            ensure!(
                unsafe { SetStdHandle(written, new.as_raw_handle() as HANDLE) } != 0,
                "couldn't write {}",
                log.display()
            );
        }
        *writing = Some(new);
        Ok(())
    }

    /// A job object holding the server and whatever it starts, which are all stopped once it's let go of: when the
    /// server is stopped or this program ends, however it ends. A release's script, which the server is, starts its VM
    /// once it has read two batch files, long after this. On Linux and a Mac they go with it already: systemd stops a
    /// unit's every process, launchd a job's process group, and the server stops when its input closes.
    pub struct Contained(HANDLE);

    impl Drop for Contained {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn contain(server: &Child) -> Result<Contained> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            ensure!(!job.is_null(), "couldn't make a job object: {}", std::io::Error::last_os_error());
            let job = Contained(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = std::mem::size_of_val(&limits) as u32;
            let limited =
                SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, &raw const limits as _, size);
            let process = server.raw_handle().context("the server has no process")?;
            ensure!(
                limited != 0 && AssignProcessToJobObject(job.0, process as HANDLE) != 0,
                "couldn't hold the server in a job object: {}",
                std::io::Error::last_os_error()
            );
            Ok(job)
        }
    }
}

/// A phone runs no server. Services on phones stay open for later.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[rustfmt::skip]
mod system {
    use super::*;

    const NONE: &str = "A server runs on Linux, macOS and Windows, and this device is none of them.";
    pub async fn start(_: &Here, _: &client::system::Program) -> Result<()> { bail!(NONE) }
    pub fn ready(_: &str) -> Result<()> { bail!(NONE) }
    pub fn bound(_: &Path) -> Result<()> { bail!(NONE) }
    pub fn log(_: &Here, _: &str) -> String { NONE.to_owned() }
}
