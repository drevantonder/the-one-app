use std::ffi::{OsStr, OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::{env, fs, mem, ptr};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, HANDLE_FLAG_INHERIT, LocalFree,
    SetHandleInformation,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, SE_FILE_OBJECT, SET_ACCESS,
    SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName, GetAppContainerFolderPath,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, DeriveCapabilitySidsFromName, EqualSid, GetAce,
    GetTokenInformation, NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows_sys::Win32::System::SystemServices::SE_GROUP_ENABLED;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};

/// The AppContainer every program behind the wall runs in, which what they all read is granted to once.
const CONTAINER: &str = "app.theone.sandbox";

pub fn main() -> i32 {
    match walled() {
        Ok(code) => code as i32,
        Err(why) => {
            eprintln!("The wall didn't start it: {why}");
            125
        }
    }
}

type Result<T> = std::result::Result<T, String>;

struct Asked {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
    env: Vec<(OsString, OsString)>,
    memory: Option<usize>,
    program: Vec<OsString>,
}

fn asked() -> Result<Asked> {
    let mut asked = Asked { read: vec![], write: vec![], env: vec![], memory: None, program: vec![] };
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{} needs a value", arg.display()));
        match arg.to_str() {
            // Windows' security functions take a path as Windows writes it, which Erlang doesn't.
            Some("--read") => asked.read.push(value()?.to_string_lossy().replace('/', "\\").into()),
            Some("--write") => asked.write.push(value()?.to_string_lossy().replace('/', "\\").into()),
            Some("--memory") => asked.memory = Some(value()?.to_str().and_then(|n| n.parse().ok()).ok_or("--memory is bytes")?),
            Some("--env") => {
                let said = value()?;
                let (name, value) = said.to_str().and_then(|said| said.split_once('=')).ok_or("--env is NAME=value")?;
                asked.env.push((name.into(), value.into()));
            }
            Some("--") => asked.program = args.by_ref().collect(),
            _ => return Err(format!("{} isn't something the wall takes", arg.display())),
        }
    }
    if asked.program.is_empty() {
        return Err("it's given no program, after --".into());
    }
    Ok(asked)
}

fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain([0]).collect()
}

fn failed(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

fn walled() -> Result<u32> {
    let asked = asked()?;
    unsafe {
        let container = container()?;
        // This run's own capability: its folder is granted to it alone.
        // Random, as the standard library's hashers are keyed from the system's random source.
        let random = std::hash::BuildHasher::hash_one(&std::hash::RandomState::new(), std::process::id());
        let run = format!("one-sandbox-{random:016x}");
        let capability = capability(&run)?;

        for path in &asked.read {
            grant(path, container, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?;
        }
        for path in &asked.write {
            grant(path, container, FILE_ALL_ACCESS)?;
        }

        // Its folder, in the AppContainer's own, with only the person and this run's capability on it.
        let folder = folder_of(container)?.join(&run);
        fs::create_dir(&folder).map_err(|error| format!("its folder: {error}"))?;
        let walled = (|| {
            own(&folder, capability)?;
            started(&asked, container, capability, &folder)
        })();
        let _ = fs::remove_dir_all(&folder);
        walled
    }
}

/// The AppContainer's SID, which it's made with the first time.
unsafe fn container() -> Result<PSID> {
    let name = wide(CONTAINER);
    let mut sid: PSID = ptr::null_mut();
    let made = unsafe {
        CreateAppContainerProfile(
            name.as_ptr(),
            wide("the One App's sandbox").as_ptr(),
            wide("Untrusted apps, behind the One App's wall").as_ptr(),
            ptr::null(),
            0,
            &mut sid,
        )
    };
    if made == (0x8007_0000u32 | ERROR_ALREADY_EXISTS) as i32 {
        let derived = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        if derived != 0 {
            return Err(format!("the AppContainer's SID: {derived:#x}"));
        }
    } else if made != 0 {
        return Err(format!("the AppContainer: {made:#x}"));
    }
    Ok(sid)
}

unsafe fn capability(name: &str) -> Result<PSID> {
    let (mut groups, mut group_count, mut capabilities, mut count) = (ptr::null_mut(), 0, ptr::null_mut(), 0);
    let derived = unsafe {
        DeriveCapabilitySidsFromName(wide(name).as_ptr(), &mut groups, &mut group_count, &mut capabilities, &mut count)
    };
    if derived == 0 || count == 0 {
        return Err(failed("its capability"));
    }
    Ok(unsafe { *capabilities })
}

unsafe fn folder_of(container: PSID) -> Result<PathBuf> {
    let mut text = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(container, &mut text) } == 0 {
        return Err(failed("the AppContainer's SID as text"));
    }
    let mut path = ptr::null_mut();
    let found = unsafe { GetAppContainerFolderPath(text, &mut path) };
    unsafe { LocalFree(text as _) };
    if found != 0 {
        return Err(format!("the AppContainer's folder: {found:#x}"));
    }
    let folder = unsafe { OsString::from_wide(std::slice::from_raw_parts(path, (0..).take_while(|&i| *path.add(i) != 0).count())) };
    Ok(folder.into())
}

fn trustee(sid: PSID) -> TRUSTEE_W {
    TRUSTEE_W {
        pMultipleTrustee: ptr::null_mut(),
        MultipleTrusteeOperation: 0,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_UNKNOWN,
        ptstrName: sid as _,
    }
}

/// Grants `sid` `access` to `path`, and what's in it, unless it has it already: each grant is made once, as it's
/// carried down to everything in the folder, which takes a while.
unsafe fn grant(path: &Path, sid: PSID, access: u32) -> Result<()> {
    let name = wide(path);
    let (mut dacl, mut descriptor): (*mut ACL, PSECURITY_DESCRIPTOR) = (ptr::null_mut(), ptr::null_mut());
    let read = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if read != ERROR_SUCCESS {
        return Err(format!("{} can't be read for its access: {}", path.display(), std::io::Error::from_raw_os_error(read as i32)));
    }
    let granted = !dacl.is_null()
        && (0..unsafe { (*dacl).AceCount } as u32).any(|index| {
            let mut ace: *mut c_void = ptr::null_mut();
            unsafe {
                GetAce(dacl, index, &mut ace) != 0
                    && (*(ace as *const ACCESS_ALLOWED_ACE)).Header.AceType == 0
                    && (*(ace as *const ACCESS_ALLOWED_ACE)).Mask & access == access
                    && EqualSid(&raw const (*(ace as *const ACCESS_ALLOWED_ACE)).SidStart as PSID, sid) != 0
            }
        });
    let set = if granted {
        ERROR_SUCCESS
    } else {
        let inheritance = if path.is_dir() { SUB_CONTAINERS_AND_OBJECTS_INHERIT } else { NO_INHERITANCE };
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: access,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: inheritance,
            Trustee: trustee(sid),
        };
        let mut new: *mut ACL = ptr::null_mut();
        let made = unsafe { SetEntriesInAclW(1, &entry, dacl, &mut new) };
        let set = if made == ERROR_SUCCESS {
            unsafe {
                SetNamedSecurityInfoW(name.as_ptr() as _, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, ptr::null_mut(), ptr::null_mut(), new, ptr::null())
            }
        } else {
            made
        };
        unsafe { LocalFree(new as _) };
        set
    };
    unsafe { LocalFree(descriptor) };
    if set != ERROR_SUCCESS {
        return Err(format!("{} couldn't be granted: {}", path.display(), std::io::Error::from_raw_os_error(set as i32)));
    }
    Ok(())
}

/// Makes `folder` the person's and this run's capability's alone, with nothing of its parent's.
unsafe fn own(folder: &Path, capability: PSID) -> Result<()> {
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(failed("this program's token"));
    }
    let mut user = [0u8; 256];
    let mut size = 0;
    let got = unsafe { GetTokenInformation(token, TokenUser, user.as_mut_ptr() as _, user.len() as u32, &mut size) };
    unsafe { CloseHandle(token) };
    if got == 0 {
        return Err(failed("who runs this"));
    }
    let person = unsafe { (*(user.as_ptr() as *const TOKEN_USER)).User.Sid };
    let entries = [person, capability].map(|sid| EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: trustee(sid),
    });
    let mut acl: *mut ACL = ptr::null_mut();
    let made = unsafe { SetEntriesInAclW(entries.len() as u32, entries.as_ptr(), ptr::null(), &mut acl) };
    if made != ERROR_SUCCESS {
        return Err(format!("its folder's access: {}", std::io::Error::from_raw_os_error(made as i32)));
    }
    let set = unsafe {
        SetNamedSecurityInfoW(
            wide(folder).as_ptr() as _,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null(),
        )
    };
    unsafe { LocalFree(acl as _) };
    if set != ERROR_SUCCESS {
        return Err(format!("its folder's access: {}", std::io::Error::from_raw_os_error(set as i32)));
    }
    Ok(())
}

/// A program's command line, each argument quoted as Windows' C runtime reads it back (`CommandLineToArgvW`).
fn command_line(program: &[OsString]) -> Vec<u16> {
    let mut line: Vec<u16> = Vec::new();
    for (index, arg) in program.iter().enumerate() {
        if index > 0 {
            line.push(b' ' as u16);
        }
        let arg: Vec<u16> = arg.encode_wide().collect();
        let plain = !arg.is_empty() && !arg.iter().any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'"' as u16);
        if plain {
            line.extend(arg);
            continue;
        }
        line.push(b'"' as u16);
        let mut slashes = 0;
        for c in arg {
            if c == b'\\' as u16 {
                slashes += 1;
            } else {
                let doubled = if c == b'"' as u16 { slashes * 2 + 1 } else { slashes };
                line.extend(std::iter::repeat_n(b'\\' as u16, doubled));
                slashes = 0;
                line.push(c);
            }
        }
        line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        line.push(b'"' as u16);
    }
    line.push(0);
    line
}

unsafe fn started(asked: &Asked, container: PSID, capability: PSID, folder: &Path) -> Result<u32> {
    // The job it's in: its memory limit, and its end once this ends, however that is.
    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() {
        return Err(failed("its job object"));
    }
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    if let Some(memory) = asked.memory {
        limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
        limits.JobMemoryLimit = memory;
    }
    let limited = unsafe {
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &raw const limits as _, mem::size_of_val(&limits) as u32)
    };
    if limited == 0 {
        return Err(failed("its job object's limits"));
    }

    // What it inherits: this program's standard input, output and error, and nothing else.
    let mut handles: Vec<HANDLE> = Vec::new();
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        let handle = unsafe { GetStdHandle(which) };
        if !handle.is_null() && handle as isize != -1 && !handles.contains(&handle) {
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
            handles.push(handle);
        }
    }

    let mut capabilities = [SID_AND_ATTRIBUTES { Sid: capability, Attributes: SE_GROUP_ENABLED as u32 }];
    let security = SECURITY_CAPABILITIES {
        AppContainerSid: container,
        Capabilities: capabilities.as_mut_ptr(),
        CapabilityCount: 1,
        Reserved: 0,
    };
    let jobs = [job];
    let mut size = 0;
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 3, 0, &mut size) };
    let mut list = vec![0u8; size];
    let attributes = list.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    let set = unsafe {
        InitializeProcThreadAttributeList(attributes, 3, 0, &mut size) != 0
            && UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize, &raw const security as _, mem::size_of_val(&security), ptr::null_mut(), ptr::null()) != 0
            && UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, jobs.as_ptr() as _, mem::size_of_val(&jobs), ptr::null_mut(), ptr::null()) != 0
            && (handles.is_empty()
                || UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, handles.as_ptr() as _, handles.len() * mem::size_of::<HANDLE>(), ptr::null_mut(), ptr::null()) != 0)
    };
    if !set {
        return Err(failed("how it's started"));
    }

    let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
    startup.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    startup.StartupInfo.hStdOutput = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    startup.StartupInfo.hStdError = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    startup.lpAttributeList = attributes;

    // Its environment: what it's given, Windows' folder, which Windows' own libraries need, and its own folder.
    let mut windows = [0u16; 260];
    let length = unsafe { GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) } as usize;
    let windows = OsString::from_wide(&windows[..length]);
    let mut environment: Vec<(OsString, OsString)> = asked.env.clone();
    environment.push(("SystemRoot".into(), windows));
    for name in ["TMPDIR", "TEMP", "TMP"] {
        environment.push((name.into(), folder.into()));
    }
    let mut block: Vec<u16> = Vec::new();
    for (name, value) in &environment {
        block.extend(name.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);

    let program = wide(&asked.program[0]);
    let mut line = command_line(&asked.program);
    let mut process: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            program.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            block.as_ptr() as _,
            wide(folder).as_ptr(),
            &raw const startup.StartupInfo,
            &mut process,
        )
    };
    if created == 0 {
        return Err(failed(&format!("{}", asked.program[0].display())));
    }
    unsafe {
        CloseHandle(process.hThread);
        WaitForSingleObject(process.hProcess, INFINITE);
        let mut code = 0;
        GetExitCodeProcess(process.hProcess, &mut code);
        CloseHandle(process.hProcess);
        // Whatever it started goes with it.
        CloseHandle(job);
        Ok(code)
    }
}
