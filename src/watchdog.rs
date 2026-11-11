//! What keeps a program of a computer's going: its watchdog, which exits it once its runtime has made no progress for
//! a while, for whatever started it to start it again (oned for the app runtime; systemd, launchd or Windows' task for
//! oned), and telling systemd it's ready.

use std::panic::Location;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::time::Duration;

/// How many times in a row the watchdog has restarted this program, kept in its home.
const RESTARTS: &str = "watchdog-restarts";

static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Notes that this program started now, for `since_start`.
pub fn started() {
    STARTED.get_or_init(std::time::Instant::now);
}

/// How long since this program started (`started`), for what waits a moment then.
pub fn since_start() -> Option<Duration> {
    STARTED.get().map(|s| s.elapsed())
}

/// Tells systemd `state`, when it runs oned as its service.
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

/// Whether a supervisor starts this program again when it exits: oned, or what starts oned. Only a program started by
/// hand on Linux, outside a systemd service, has none.
fn supervised() -> bool {
    !cfg!(target_os = "linux") || unit().is_some()
}

/// The systemd service this program runs in, if it's one's: not a program started from a shell in one.
fn unit() -> Option<String> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let unit = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?.rsplit('/').next()?.to_string();
    (unit.ends_with(".service") && std::env::var_os("INVOCATION_ID").is_some()).then_some(unit)
}

/// What's held the node's who's who since when (`Node::with`), which a stuck runtime is usually waiting for: what says why
/// when it can't be seen from outside.
static HELD_AT: AtomicPtr<Location<'static>> = AtomicPtr::new(std::ptr::null_mut());
static HELD_SINCE: AtomicU64 = AtomicU64::new(0);

/// While it lives, says who has the node's who's who, for a stall to name.
pub struct Held;

impl Held {
    #[track_caller]
    pub fn new() -> Held {
        HELD_SINCE.store(STARTED.get().map_or(0, |s| s.elapsed().as_secs()), Ordering::Relaxed);
        HELD_AT.store(std::panic::Location::caller() as *const _ as *mut _, Ordering::Relaxed);
        Held
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        HELD_AT.store(std::ptr::null_mut(), Ordering::Relaxed);
    }
}

/// What a stuck runtime was doing, as far as can be seen from outside it: what held its who's who, how many tasks it
/// had and threads it was waiting on, and, on a Mac, every thread's stack, kept in the node's log folder.
fn stuck_on(runtime: &tokio::runtime::Handle, now: u64) -> String {
    let at = HELD_AT.load(Ordering::Relaxed);
    // Set only from `Location::caller()`, which is 'static, and never freed.
    let held = (!at.is_null()).then(|| unsafe { &*at }).map_or("nothing".to_string(), |l| format!("{l} for {} seconds", now.saturating_sub(HELD_SINCE.load(Ordering::Relaxed))));
    let metrics = runtime.metrics();
    let mut said = format!("Who's who was held by {held}; {} tasks alive on {} workers, {} queued", metrics.num_alive_tasks(), metrics.num_workers(), metrics.global_queue_depth());
    if cfg!(target_os = "macos") {
        let file = platform::home().join("logs").join("stall.sample");
        if std::process::Command::new("sample").arg(std::process::id().to_string()).args(["2", "-file"]).arg(&file).output().is_ok_and(|o| o.status.success()) {
            said.push_str(&format!("; every thread's stack is in {}", file.display()));
        }
    }
    said
}

/// A node whose runtime has made no progress for a minute says so and exits, for its supervisor to start it again. A node
/// whose every thread is stuck still listens, and what's asked of it would otherwise wait unanswered, every agent and the
/// CLI with it, until someone restarts it by hand. Progress is a task on the runtime that ticks every few seconds, not a
/// request answered in time: a busy node (an update building on its machine) answers slowly, and is not stuck. Starting a
/// node costs its machine enough to stall what comes next, so each restart in a row waits longer before the next, up to
/// `MOST_STUCK_FOR`: a real hang still recovers, and a node that stays up (`SETTLED`) has none in a row.
pub fn watch() {
    if !supervised() {
        return;
    }
    const STUCK_FOR: u64 = 60;
    const MOST_STUCK_FOR: u64 = 480;
    const SETTLED: u64 = 15 * 60;
    const CHECK_EVERY: Duration = Duration::from_secs(10);
    const LATE: Duration = Duration::from_secs(5);
    let runtime = tokio::runtime::Handle::current();
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
    let restarted = platform::home().join(RESTARTS);
    let mut in_a_row: u32 = std::fs::read_to_string(&restarted).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(0);
    std::thread::spawn(move || {
        let mut counting_from = 0;
        loop {
            let asleep = std::time::Instant::now();
            std::thread::sleep(CHECK_EVERY);
            let late = asleep.elapsed().saturating_sub(CHECK_EVERY);
            // This thread does nothing but wake, so its being late means the machine kept the whole process off its CPUs
            // or disk: the runtime isn't stuck, and starting again would only add to the load.
            if late >= LATE {
                counting_from = up();
                tracing::warn!("this node was held up for {} seconds by the machine it's on", late.as_secs());
                continue;
            }
            if in_a_row > 0 && up() >= SETTLED {
                in_a_row = 0;
                let _ = std::fs::remove_file(&restarted);
            }
            let silent = up().saturating_sub(beat.load(std::sync::atomic::Ordering::SeqCst).max(counting_from));
            if silent >= (STUCK_FOR << in_a_row.min(3)).min(MOST_STUCK_FOR) {
                tracing::error!("this node's runtime has done nothing for {silent} seconds: exiting, to be started again. {}", stuck_on(&runtime, up()));
                ::insights::Ended::Watchdog.write(&platform::home());
                let _ = std::fs::write(&restarted, (in_a_row + 1).to_string());
                std::process::exit(1);
            }
        }
    });
}

