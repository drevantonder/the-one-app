//! A node taking over from the one before it with no gap, for an update: the running node starts the new build with
//! its listening socket, the new one starts, loads and warms its apps, then says it's
//! ready (and to systemd, that it's the service's main process now), and the old one stops taking connections, lets
//! what it's answering finish, and exits. What reaches the address meanwhile waits in the socket's queue, never refused.
//! Only on Linux, and under systemd only where the service lets other processes tell it its main pid
//! (`NotifyAccess=all`, as install.sh writes it); elsewhere a node exits and its supervisor starts the new one.

use std::time::Duration;

use anyhow::{Context, Result, bail};

/// The arg a node is started with to take over: `--handoff=<listening fd>,<ready fd>,<runtime folder>`.
const ARG: &str = "--handoff=";
/// The two folders the node's app runtime alternates between, so the old one's keeps running while the new one starts.
const RUNTIMES: [&str; 2] = ["runtime", "runtime-next"];
/// How long the new build may take to be ready, and the old one to finish what it's answering: as long as an app may take.
const READY_WITHIN: Duration = Duration::from_secs(120);
const DRAIN_WITHIN: Duration = Duration::from_secs(30);

static STOPPING: std::sync::LazyLock<tokio::sync::watch::Sender<bool>> = std::sync::LazyLock::new(|| tokio::sync::watch::channel(false).0);
static WANTED: tokio::sync::Notify = tokio::sync::Notify::const_new();
static ABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static RUNTIME: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// How long since this process's node started serving (`handed`, which `serve` calls), for what waits a moment then.
pub fn since_start() -> Option<Duration> {
    STARTED.get().map(|s| s.elapsed())
}

/// What a node taking over was handed: the socket to listen on, and where to say it's ready.
pub struct Handed {
    pub listener: std::net::TcpListener,
    ready: std::fs::File,
}

pub fn handed(args: &[String]) -> Result<Option<Handed>> {
    STARTED.get_or_init(std::time::Instant::now);
    let Some(arg) = args.iter().find_map(|a| a.strip_prefix(ARG)) else {
        RUNTIME.get_or_init(|| RUNTIMES[0]);
        return Ok(None);
    };
    let mut parts = arg.split(',');
    let (Some(listen), Some(ready), Some(runtime)) = (parts.next(), parts.next(), parts.next()) else { bail!("{ARG}{arg} isn't <fd>,<fd>,<folder>") };
    RUNTIME.get_or_init(|| RUNTIMES.into_iter().find(|r| *r == runtime).unwrap_or(RUNTIMES[0]));
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        let (listen, ready): (i32, i32) = (listen.parse()?, ready.parse()?);
        // Not passed on to what this node starts.
        for fd in [listen, ready] {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        // SAFETY: the node before handed these to this process alone, and nothing here has opened them.
        let listener = unsafe { std::net::TcpListener::from_raw_fd(listen) };
        listener.set_nonblocking(true)?;
        let ready = unsafe { std::fs::File::from_raw_fd(ready) };
        Ok(Some(Handed { listener, ready }))
    }
    #[cfg(not(unix))]
    bail!("only a node on Linux takes over from another")
}

/// The folder in the node's home its app runtime keeps its files in.
pub fn runtime() -> &'static str {
    RUNTIME.get().copied().unwrap_or(RUNTIMES[0])
}

impl Handed {
    /// Says this node is ready to take over: to systemd, that it's the service's main process, then to the node before.
    pub fn ready(mut self) {
        use std::io::Write;
        notify(&format!("MAINPID={}\nREADY=1", std::process::id()));
        let _ = self.ready.write_all(b"1");
    }
}

/// Tells systemd `state`, when it runs this node as a service that listens.
pub fn notify(#[cfg_attr(not(target_os = "linux"), allow(unused_variables))] state: &str) {
    #[cfg(target_os = "linux")]
    if let Some(at) = std::env::var_os("NOTIFY_SOCKET") {
        use std::os::linux::net::SocketAddrExt;
        let at = at.to_string_lossy().into_owned();
        let addr = match at.strip_prefix('@') {
            Some(name) => std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes()),
            None => std::os::unix::net::SocketAddr::from_pathname(&at),
        };
        let sent = std::os::unix::net::UnixDatagram::unbound().and_then(|s| s.send_to_addr(state.as_bytes(), &addr?));
        if let Err(err) = sent {
            tracing::warn!("couldn't tell systemd {state:?}: {err}");
        }
    }
}

/// Whether this node can hand over to a new build rather than exit for its supervisor to start it: on Linux, either
/// under no supervisor, or under systemd where it may tell it the new main pid.
fn possible() -> bool {
    cfg!(target_os = "linux") && (unit().is_none() || std::env::var_os("NOTIFY_SOCKET").is_some())
}

/// The systemd service this node runs as, if it's one's: not a program started from a shell in one.
fn unit() -> Option<String> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let unit = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?.rsplit('/').next()?.to_string();
    (unit.ends_with(".service") && std::env::var_os("INVOCATION_ID").is_some()).then_some(unit)
}

/// Whether a supervisor starts this node again when it exits.
pub fn supervised() -> bool {
    !cfg!(target_os = "linux") || unit().is_some()
}

/// A node whose runtime has made no progress for a minute says so and exits, for its supervisor to start it again. A node
/// whose every thread is stuck still listens, and what's asked of it would otherwise wait unanswered, every agent and the
/// CLI with it, until someone restarts it by hand. Progress is a task on the runtime that ticks every few seconds, not a
/// request answered in time: a busy node (an update building on its machine) answers slowly, and is not stuck.
pub fn watch() {
    if !supervised() {
        return;
    }
    const STUCK_FOR: u64 = 60;
    let beat = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let up = || STARTED.get().map_or(0, |s| s.elapsed().as_secs());
    tokio::spawn({
        let beat = beat.clone();
        async move {
            loop {
                beat.store(up(), std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    });
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(10));
            if stopped() {
                return;
            }
            let silent = up().saturating_sub(beat.load(std::sync::atomic::Ordering::SeqCst));
            if silent >= STUCK_FOR {
                tracing::error!("this node's runtime has done nothing for {silent} seconds: exiting, to be started again");
                crate::insights::Ended::Watchdog.write(&crate::home());
                std::process::exit(1);
            }
        }
    });
}

/// Lets `wanted` hand over from now on, once the node serves.
pub fn able() {
    ABLE.store(possible(), std::sync::atomic::Ordering::SeqCst);
}

/// Restarts the node on the program now at its path: handed over, or, where that can't be, exited for its supervisor
/// to start again. One that will hand over but doesn't serve yet does nothing: asked again once it does (as
/// `Node::pull_loop` asks), it hands over, rather than exiting with nothing to start it again. → whether it asked.
pub fn restart() -> bool {
    if ABLE.load(std::sync::atomic::Ordering::SeqCst) {
        WANTED.notify_one();
        true
    } else if possible() {
        false
    } else {
        std::process::exit(0);
    }
}

/// Waits until the node is asked to restart (an update, or SIGHUP).
pub async fn wanted() {
    WANTED.notified().await
}

/// Waits until this node is stopping for the one taking over: long waits (event streams, services' asks for calls)
/// end then, so their clients ask the new one at once.
pub async fn stopping() {
    let mut rx = STOPPING.subscribe();
    let _ = rx.wait_for(|s| *s).await;
}

pub fn stopped() -> bool {
    *STOPPING.borrow()
}

/// Starts the program at the node's path, `args` and all, to take over `listener`, and waits until it's ready. Then
/// this node is to stop taking connections, finish what it's answering, and exit.
#[cfg(target_os = "linux")]
pub async fn hand_over(listener: i32, args: &[String]) -> Result<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    // Closed on exec, on Linux and macOS alike, but for the end handed to the new build.
    let (read, write) = std::io::pipe().context("pipe")?;
    let next = if runtime() == RUNTIMES[0] { RUNTIMES[1] } else { RUNTIMES[0] };
    let exe = crate::this_program()?;
    let mut command = std::process::Command::new(&exe);
    // Started by the name this one was (oned), which says what it is.
    if let Some(name) = std::env::args_os().next() {
        command.arg0(name);
    }
    command.args(args.iter().filter(|a| !a.starts_with(ARG))).arg(format!("{ARG}{listener},{},{next}", write.as_raw_fd()));
    let keep = [listener, write.as_raw_fd()];
    // SAFETY: only async-signal-safe calls, between fork and exec.
    unsafe {
        command.pre_exec(move || {
            for fd in keep {
                if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn().with_context(|| format!("start {}", exe.display()))?;
    drop(write);
    let said = tokio::time::timeout(
        READY_WITHIN,
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut byte = [0u8; 1];
            (&read).read(&mut byte).map(|n| n == 1)
        }),
    )
    .await;
    if !matches!(said, Ok(Ok(Ok(true)))) {
        let _ = child.kill();
        let _ = child.wait();
        bail!("the new build didn't start: see its log");
    }
    // Reaped by whoever adopts it once this exits; never waited on here.
    std::mem::forget(child);
    if unit().is_some() {
        main_moved().await?;
    }
    let _ = STOPPING.send(true);
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub async fn hand_over(_listener: i32, _args: &[String]) -> Result<()> {
    bail!("only a node on Linux takes over from another")
}

#[cfg(target_os = "linux")]
/// Waits until systemd takes the node taking over as the service's main process, as it said it is: this one exiting
/// before then would stop the service, the new one with it.
async fn main_moved() -> Result<()> {
    let unit = unit().context("this node isn't a systemd service's")?;
    let me = std::process::id().to_string();
    for _ in 0..50 {
        let out = tokio::process::Command::new("systemctl").args(["show", "-p", "MainPID", "--value", &unit]).output().await?;
        let main = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !main.is_empty() && main != "0" && main != me {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("systemd didn't take the new build as {unit}'s main process: its unit needs NotifyAccess=all")
}

pub fn drain_within() -> Duration {
    DRAIN_WITHIN
}
