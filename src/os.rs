//! What differs between the operating systems this program runs on, so the rest of it doesn't have to know: which name
//! it was started by, stopping what it started, private files, links, replacing the running program and a disk's space.

use std::path::Path;
use std::process::{Child, Command};

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
    use crate::config::CLI;
    let dir = exe.parent().ok_or_else(|| std::io::Error::other("this program is in no folder"))?;
    let name = |n: String| dir.join(format!("{n}{}", std::env::consts::EXE_SUFFIX));
    let oned = name(format!("{CLI}d"));
    let mut names = vec![name(crate::git_wire::helper()), name(crate::oned::onet()), name(CLI.to_string()), exe.to_path_buf()];
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
