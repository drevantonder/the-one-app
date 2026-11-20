//! How much memory the app uses, as its system counts it, and where that is, told to Telemetry once what's on
//! screen has settled after it changes: `one.client.memory` is the process's, and `one.client.memory.<part>` its
//! parts' (`renderer::platform::Windows::memory`), each in MB with the state it was in (`one.state`: `background`
//! with nothing on screen, `launcher` while it's summoned, `windows` with an app's window open) and how many windows
//! were on screen (`one.windows`), so `telemetry metric` shows each state's over time and a regression stands out.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use renderer::platform::Native;
use serde_json::{Value, json};

/// How long after a window shows, goes or opens its memory is measured: once what it let go of is gone, and its JS,
/// still a moment, has been collected (`renderer::js`'s `IDLE`, 250 ms).
const SETTLED: Duration = Duration::from_secs(5);

/// How many times what's on screen has changed: a change measured only once nothing's changed since.
static CHANGES: AtomicU64 = AtomicU64::new(0);

/// What's on screen changed: it's measured once it's settled.
pub(crate) fn changed() {
    let change = CHANGES.fetch_add(1, Ordering::Relaxed) + 1;
    crate::spawn(async move {
        tokio::time::sleep(SETTLED).await;
        if CHANGES.load(Ordering::Relaxed) == change {
            measure();
        }
    });
}

fn measure() {
    let Ok(native) = Native::get() else { return };
    native.run(|windows| {
        let parts = windows.memory();
        let all = parts["windows"].as_array().cloned().unwrap_or_default();
        let shown: Vec<&Value> = all.iter().filter(|window| window["on_screen"] == true).collect();
        let state = if shown.iter().any(|window| window["panel"] == false) {
            "windows"
        } else if shown.is_empty() {
            "background"
        } else {
            "launcher"
        };
        let sum = |key: &str| all.iter().flat_map(|window| window["renderers"].as_array().cloned().unwrap_or_default()).map(|renderer| renderer[key].as_u64().unwrap_or(0)).sum::<u64>();
        let surfaces = sum("surface");
        let textures = sum("kept") + sum("backdrop") + sum("rasters") + sum("tiles") + sum("buffer") + parts["atlas"].as_u64().unwrap_or(0);
        let js: u64 = all.iter().map(|window| window["js"].as_u64().unwrap_or(0)).sum();
        // The GPU's allocator's, where its backend says (not Metal, whose textures a Mac counts as their own).
        let gpu = parts["gpu"]["reserved"].as_u64();
        let process = process();
        let mb = |bytes: u64| (bytes as f64 / 1e5).round() / 10.0;
        let said = |bytes: Option<u64>| bytes.map_or("unknown".to_owned(), |bytes| format!("{} MB", mb(bytes)));
        eprintln!(
            "the app's memory, {state} with {} windows on screen of {}: {} all told; its windows' surfaces {}, textures {}, the GPU's allocator {}, JS {}; {parts}",
            shown.len(),
            all.len(),
            said(process),
            said(Some(surfaces)),
            said(Some(textures)),
            said(gpu),
            said(Some(js)),
        );
        let Ok(views) = crate::views() else { return };
        let attributes = json!({"one.state": state, "one.windows": shown.len()});
        let records = [("one.client.memory", process), ("one.client.memory.surfaces", Some(surfaces)), ("one.client.memory.textures", Some(textures)), ("one.client.memory.gpu", gpu), ("one.client.memory.js", Some(js))];
        for (name, bytes) in records {
            if let Some(bytes) = bytes {
                views.client.report(json!({"name": name, "value": mb(bytes), "attributes": attributes}));
            }
        }
    });
}

/// The process's memory as its system counts it against it: Windows' private bytes (Task Manager's Memory), a Mac's
/// and an iPhone's footprint (Activity Monitor's Memory, `footprint`'s), and Linux's and Android's private pages.
#[cfg(windows)]
fn process() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // Safety: the counters are the size said, and the process's own handle needs no closing.
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(), size) }.ok()?;
    Some(counters.PrivateUsage as u64)
}

#[cfg(target_vendor = "apple")]
fn process() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // Safety: the buffer is a `rusage_info_v2`, the flavour asked for.
    let read = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, usage.as_mut_ptr().cast()) };
    // Safety: it's filled in where it says it read.
    (read == 0).then(|| unsafe { usage.assume_init() }.ri_phys_footprint)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn process() -> Option<u64> {
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").ok()?;
    let kb = |field: &str| rollup.lines().find_map(|line| line.strip_prefix(field)?.trim().strip_suffix("kB")?.trim().parse::<u64>().ok());
    Some((kb("Private_Clean:")? + kb("Private_Dirty:")?) * 1024)
}
