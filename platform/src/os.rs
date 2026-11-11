//! What differs between the operating systems this program runs on, so the rest of it doesn't have to know: which name
//! it was started by, stopping what it started, private files, links, replacing the running program, a disk's space,
//! telling systemd it's ready, and running the tools a build uses (cargo, Go, git) wherever this machine has them.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail, ensure};

/// The name this program was started by, without Windows' `.exe`: it says what the program is to be (`oned`, git's helper).
pub fn invoked_as() -> Option<String> {
    let first = std::env::args_os().next()?;
    let name = Path::new(&first).file_name()?.to_string_lossy().into_owned();
    Some(name.strip_suffix(std::env::consts::EXE_SUFFIX).unwrap_or(&name).to_string())
}

/// This program, to be started as `name`.
#[cfg(unix)]
pub fn named(name: &str) -> std::io::Result<Command> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(crate::this_program()?);
    command.arg0(name);
    Ok(command)
}

/// This program, to be started as `name`: its link of that name beside it, as Windows can't give a program another
/// first argument, made the first time (`replace_program` keeps it the same program after).
#[cfg(windows)]
pub fn named(name: &str) -> std::io::Result<Command> {
    let exe = crate::this_program()?;
    let link = exe.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if !link.exists() {
        std::fs::hard_link(&exe, &link)?;
    }
    Ok(Command::new(link))
}

/// Asks a process this one started to stop, for it to end what it runs first.
#[cfg(unix)]
pub fn ask_to_stop(child: &mut Child) {
    // SAFETY: a signal to a child of this process, which it hasn't reaped.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
}

/// Asks a process this one started to stop, by closing its stdin, which it watches: Windows has no signal a process
/// outside its console can send.
#[cfg(windows)]
pub fn ask_to_stop(child: &mut Child) {
    drop(child.stdin.take());
}

/// Asks a process this one didn't start to stop: on Windows, which can't ask one it didn't start, ends it.
pub fn ask_pid_to_stop(pid: i32) {
    #[cfg(unix)]
    // SAFETY: only a signal, to a process the caller checked is this program's (`is_program`).
    unsafe {
        libc::kill(pid, libc::SIGTERM)
    };
    #[cfg(windows)]
    kill(pid as u32);
}

/// Ends a process at once.
#[cfg(unix)]
pub fn kill(pid: u32) {
    // SAFETY: only a signal; a pid that's gone is an error nothing reads.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
}

#[cfg(windows)]
pub fn kill(pid: u32) {
    let _ = Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
}

/// Asks a process group to stop: `-pid` its leader's, 0 this process's own. Only a throwaway network has them, which
/// runs on the machines that build, and none of those is Windows.
pub fn ask_group_to_stop(#[cfg_attr(windows, allow(unused_variables))] pid: i32) {
    // SAFETY: only a signal.
    #[cfg(unix)]
    unsafe {
        libc::kill(pid, libc::SIGTERM)
    };
}

/// Whether the process `pid` is a program named `program`, as this machine says.
#[cfg(unix)]
pub fn is_program(pid: u32, program: &str) -> bool {
    let out = Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).output().ok();
    out.is_some_and(|o| Path::new(String::from_utf8_lossy(&o.stdout).trim()).file_name().is_some_and(|n| n.to_string_lossy() == program))
}

#[cfg(windows)]
pub fn is_program(pid: u32, program: &str) -> bool {
    let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output().ok();
    let name = program.strip_suffix(std::env::consts::EXE_SUFFIX).unwrap_or(program).to_lowercase();
    out.is_some_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains(&format!("\"{name}{}\"", std::env::consts::EXE_SUFFIX)))
}

/// Ends everything this process starts when it ends, however it does, as `PR_SET_PDEATHSIG` does for each child on
/// Linux: every process it starts from now on is in a job that's closed when the last handle to it goes, and this one
/// is never closed.
#[cfg(windows)]
pub fn children_end_with_me() -> anyhow::Result<()> {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: a new job with a fully written limit struct of the size given, and this process's own pseudo-handle.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        anyhow::ensure!(!job.is_null(), "making a job: {}", std::io::Error::last_os_error());
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        // Breakaway is for what must outlive this process: an update it starts.
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        anyhow::ensure!(SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const limits).cast(), size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) != 0, "setting its limits: {}", std::io::Error::last_os_error());
        anyhow::ensure!(AssignProcessToJobObject(job, GetCurrentProcess()) != 0, "joining it: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

/// Options for writing a file only this machine's user reads. On Windows that's where it's kept: under the user's
/// profile, whose permissions every file there takes on.
pub fn private_file() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

/// Sets a path's Unix permissions; on Windows, what's under the user's profile is already theirs alone.
pub fn set_mode(#[cfg_attr(windows, allow(unused_variables))] path: &Path, #[cfg_attr(windows, allow(unused_variables))] mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode))?;
    Ok(())
}

/// Puts a link to the program `original` at `link`: a hard link on Windows, where a symbolic one needs developer mode.
pub fn link_program(original: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(original, link);
    #[cfg(windows)]
    return std::fs::hard_link(original, link);
}

/// Puts the program at `new` where `exe` is, in one step, so nothing ever finds it half written or missing.
#[cfg(unix)]
pub fn replace_program(new: &Path, exe: &Path) -> std::io::Result<()> {
    std::fs::rename(new, exe)
}

/// Puts the program at `new` at `exe` and at its other names beside it, each hard links of one file (`link_program`).
/// Windows only refuses to take away the last name of a program that's running, so the file being replaced first gets
/// one more, `*.old`, which goes once nothing runs it (`remove_replaced`). `<cli>d`'s name goes last, so a crash partway
/// leaves the old build running, and the next update check finishes the job.
#[cfg(windows)]
pub fn replace_program(new: &Path, exe: &Path) -> std::io::Result<()> {
    use ::custom_build::config::CLI;
    let dir = exe.parent().ok_or_else(|| std::io::Error::other("this program is in no folder"))?;
    let name = |n: String| dir.join(format!("{n}{}", std::env::consts::EXE_SUFFIX));
    let oned = name(format!("{CLI}d"));
    let mut names = vec![name(::custom_build::programs::git_helper()), name(::custom_build::programs::onet()), name(CLI.to_string()), exe.to_path_buf()];
    names.retain(|path| path.exists() && *path != oned);
    names.dedup();
    names.extend(oned.exists().then_some(oned));
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    std::fs::hard_link(exe, exe.with_extension(format!("{when}.old")))?;
    for path in &names {
        let next = path.with_extension("next");
        let _ = std::fs::remove_file(&next);
        std::fs::hard_link(new, &next)?;
        std::fs::rename(&next, path)?;
    }
    std::fs::remove_file(new)
}

/// Removes what updates left beside the program at `exe` (`replace_program`): whichever nothing runs any more.
#[cfg(windows)]
pub fn remove_replaced(exe: &Path) {
    let Some(dir) = exe.parent() else { return };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.path().extension().is_some_and(|e| e == "old") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// What tells a file from the one that replaced it at its path.
pub fn file_id(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    return std::os::unix::fs::MetadataExt::ino(meta);
    #[cfg(windows)]
    return std::os::windows::fs::MetadataExt::last_write_time(meta);
}

/// How much of the disk `path` is on is used, and how much more this user may use, in bytes.
#[cfg(unix)]
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path, and a struct statvfs fills in.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = st.f_frsize as u64;
    Some(((st.f_blocks as u64).saturating_sub(st.f_bfree as u64) * block, st.f_bavail as u64 * block))
}

#[cfg(windows)]
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: a NUL-terminated path, and three numbers it fills in.
    let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
    (ok != 0).then(|| (total.saturating_sub(free), avail))
}

/// When this program is asked to stop: Ctrl-C, or SIGTERM from its supervisor or an update, each listened for from when
/// it's called.
pub fn stop_asked() -> anyhow::Result<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (mut term, mut interrupt) = (signal(SignalKind::terminate())?, signal(SignalKind::interrupt())?);
        Ok(Box::pin(async move {
            tokio::select! {
                _ = term.recv() => {}
                _ = interrupt.recv() => {}
            }
        }))
    }
    #[cfg(not(unix))]
    Ok(Box::pin(async {
        let _ = tokio::signal::ctrl_c().await;
    }))
}

/// Makes `command`, a workerd, serve each of `listening`, a socket already listening, as its config's socket of that
/// name, in place of the address there: no one else can take the port before workerd does, and what reaches it waits
/// for workerd, while it starts again too, rather than being refused. Only that child keeps them open past exec, not
/// whatever else this process starts meanwhile. On Linux the child dies with this process.
#[cfg_attr(windows, allow(unused_variables))]
pub fn child(command: &mut std::process::Command, listening: &[(&str, &std::net::TcpListener)]) {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        let fds: Vec<i32> = listening.iter().map(|(_, l)| l.as_raw_fd()).collect();
        for (name, listener) in listening {
            command.arg(format!("--socket-fd={name}={}", listener.as_raw_fd()));
        }
        // SAFETY: only async-signal-safe calls, between fork and exec.
        unsafe {
            command.pre_exec(move || {
                for &fd in &fds {
                    if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                #[cfg(target_os = "linux")]
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
}

/// Starts `command` serving `listener`, a socket already listening, given it as `--listen-socket=<it>`: no other program
/// can take its port first, and what reaches it waits for the child, while it starts again too, rather than being
/// refused. Only that child keeps it, not whatever else this process starts meanwhile, which could answer in its place.
pub fn spawn_listening(command: &mut Command, listener: &std::net::TcpListener) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        let fd = listener.as_raw_fd();
        command.arg(format!("--listen-socket={fd}"));
        // SAFETY: only an async-signal-safe call, between fork and exec.
        unsafe {
            command.pre_exec(move || if libc::fcntl(fd, libc::F_SETFD, 0) == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) });
        }
        command.spawn()
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation};
        let socket = listener.as_raw_socket();
        command.arg(format!("--listen-socket={socket}"));
        // Windows hands a child every inheritable handle: the socket is one only while this child is started.
        let inherit = |on: bool| {
            // SAFETY: a socket this process holds open, which is a handle.
            if unsafe { SetHandleInformation(socket as HANDLE, HANDLE_FLAG_INHERIT, if on { HANDLE_FLAG_INHERIT } else { 0 }) } == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
        };
        inherit(true)?;
        let spawned = command.spawn();
        inherit(false)?;
        spawned
    }
}

/// The socket this program's parent handed it (`spawn_listening`), given what followed `--listen-socket=`.
pub fn handed_listener(socket: &str) -> anyhow::Result<std::net::TcpListener> {
    let socket = socket.parse().map_err(|_| anyhow::anyhow!("--listen-socket={socket} isn't a socket's number: only the program that starts this one gives it"))?;
    // SAFETY: the parent handed it open and listening, for this program alone.
    #[cfg(unix)]
    let listener = unsafe { <std::net::TcpListener as std::os::fd::FromRawFd>::from_raw_fd(socket) };
    #[cfg(windows)]
    let listener = unsafe { <std::net::TcpListener as std::os::windows::io::FromRawSocket>::from_raw_socket(socket) };
    listener.set_nonblocking(true)?;
    Ok(listener)
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

/// A tool by its name, or where it's kept when a shell doesn't have it on its PATH: `probe` is an argument that makes it
/// answer, and `kept` where it is under the home folder.
fn tool(name: &str, probe: &str, kept: &str) -> PathBuf {
    let kept = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(kept);
    if succeeds(Command::new(name).arg(probe)) || !kept.exists() { name.into() } else { kept }
}

/// Cargo, also where rustup puts it, with sccache findable too: `.cargo/config.toml` names it bare, and a shell without it on
/// its PATH (the ssh that runs a Linux device's update) gets it where mise's shims put it.
pub fn cargo() -> Command {
    cargo_behind(None)
}

/// `cargo()` at a lower priority.
pub fn nice_cargo() -> Command {
    cargo_behind(Some("nice"))
}

fn cargo_behind(wrapper: Option<&str>) -> Command {
    let cargo = tool("cargo", "--version", ".cargo/bin/cargo");
    let mut command = match wrapper {
        Some(wrapper) => {
            let mut command = Command::new(wrapper);
            command.arg(cargo);
            command
        }
        None => Command::new(cargo),
    };
    if let Some(dir) = tool("sccache", "--version", ".local/share/mise/shims/sccache").parent().filter(|dir| !dir.as_os_str().is_empty()) {
        let path = std::env::var_os("PATH").unwrap_or_default();
        command.env("PATH", std::env::join_paths(std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&path))).unwrap_or(path));
    }
    command
}

/// Go, also where mise's shims put it.
pub fn go() -> PathBuf {
    tool("go", "version", ".local/share/mise/shims/go")
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    ensure!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn quietly(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("run {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{} failed:\n{}{}", said(cmd), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

pub fn show(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("run {:?}", cmd.get_program()))?;
    ensure!(status.success(), "{} failed", said(cmd));
    Ok(())
}

pub fn succeeds(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn said(cmd: &Command) -> String {
    format!("{:?} {}", cmd.get_program(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "))
}
