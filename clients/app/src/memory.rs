//! How much memory the app uses, as its system counts it, and where that is, told to Telemetry once what's on
//! screen has settled after it changes: `one.client.memory` is the process's, and `one.client.memory.<part>` its
//! parts', each in MB: what the GPU holds for it (`gpu`, `renderer::render::Context::memory`), and of that its
//! windows' surfaces and textures (`renderer::platform::Windows::memory`); all the graphics memory the system counts
//! against it (`graphics`: the GPU's, and its system's own layers' and web view's, a Mac's and an iPhone's); what
//! its heap holds (`heap`: Rust's, QuickJS's and the system's frameworks', where its allocator says), and of that its
//! pages' JS (`js`); what's left is its code and its threads' stacks; and how many command encoders the GPU holds
//! (`one.client.gpu.encoders`), each with the state it was in (`one.state`: `background`
//! with nothing on screen, `launcher` while it's summoned, `windows` with an app's window open) and how many windows
//! were on screen (`one.windows`) and its system (`os.type`, as Telemetry adds a metric's points up whichever device
//! they're from), so `telemetry metric` shows each state's over time, and over its state's budget
//! (`budget`) an error (`one.client.memory.over`), as is a page's JS over a quarter of what it may have
//! (`one.client.memory.js.over`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use renderer::platform::Native;
use serde_json::{Value, json};

/// What the app may hold in each state besides its windows' surfaces, whose images are as big as its windows and
/// display make them, in MB: over it, Telemetry hears an error, which its `error` alert tells of the first time in 14
/// days (numbers aside, so once a state), as a regression. Each is a little over what the app held as these were set
/// (20 Nov: a Mac's 117 MB in the background, a Surface's 137 MB and a Mac's 116 to 250 MB with a window open).
fn budget(state: &str) -> u64 {
    match state {
        "background" => 150,
        "launcher" => 200,
        _ => 250,
    }
}

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
        let gpu = parts["gpu"]["reserved"].as_u64();
        let (graphics, heap) = (graphics(), heap());
        let encoders = parts["gpu"]["encoders"].as_u64().unwrap_or(0);
        let process = process();
        let mb = |bytes: u64| (bytes as f64 / 1e5).round() / 10.0;
        let said = |bytes: Option<u64>| bytes.map_or("unknown".to_owned(), |bytes| format!("{} MB", mb(bytes)));
        eprintln!(
            "the app's memory, {state} with {} windows on screen of {}: {} all told; the GPU {} (its windows' surfaces {}, textures {}) and {encoders} command encoders, graphics {}, the heap {} (JS {}); {parts}",
            shown.len(),
            all.len(),
            said(process),
            said(gpu),
            said(Some(surfaces)),
            said(Some(textures)),
            said(graphics),
            said(heap),
            said(Some(js)),
        );
        let Ok(views) = crate::views() else { return };
        let attributes = json!({"one.state": state, "one.windows": shown.len(), "os.type": std::env::consts::OS});
        let records = [("one.client.memory", process), ("one.client.memory.surfaces", Some(surfaces)), ("one.client.memory.textures", Some(textures)), ("one.client.memory.gpu", gpu), ("one.client.memory.graphics", graphics), ("one.client.memory.heap", heap), ("one.client.memory.js", Some(js))];
        for (name, bytes) in records {
            if let Some(bytes) = bytes {
                views.client.report(json!({"name": name, "value": mb(bytes), "attributes": attributes}));
            }
        }
        views.client.report(json!({"name": "one.client.gpu.encoders", "value": encoders, "attributes": attributes}));
        if let Some(process) = process
            && mb(process.saturating_sub(surfaces)) > budget(state) as f64
        {
            let error = format!("the app holds {} MB {state} beside its windows' surfaces, over its {} MB budget", mb(process.saturating_sub(surfaces)), budget(state));
            views.client.report(json!({"name": "one.client.memory.over", "error": error, "attributes": attributes}));
        }
        // A page's JS holding a quarter of all it may, which is far more than a page holds, so a leak shows long before
        // a page runs out (`renderer::js::memory_limit`).
        let limit = renderer::js::memory_limit() as u64;
        for window in all.iter().filter(|window| window["js"].as_u64().is_some_and(|js| js > limit / 4)) {
            let page = window["entrypoint"].as_str().unwrap_or_default();
            let error = format!("{page}'s JS holds {} MB, over a quarter of the {} MB a page may have", mb(window["js"].as_u64().unwrap_or(0)), mb(limit));
            views.client.report(json!({"name": "one.client.memory.js.over", "error": error, "attributes": attributes}));
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

/// All the graphics memory the system counts against the process (its ledger's), the GPU's and its frameworks'
/// layers': a Mac's and an iPhone's, where it's what footprint calls "IOAccelerator (graphics)" and "IOSurface".
#[cfg(target_vendor = "apple")]
fn graphics() -> Option<u64> {
    use mach2::task_info::{TASK_VM_INFO, task_vm_info};
    let mut info = task_vm_info::default();
    let mut count = (std::mem::size_of::<task_vm_info>() / std::mem::size_of::<mach2::vm_types::natural_t>()) as u32;
    // Safety: the info is the size said, and the task is this one, which needs no releasing.
    let read = unsafe { mach2::task::task_info(mach2::traps::mach_task_self(), TASK_VM_INFO, (&raw mut info).cast(), &mut count) };
    let bytes = info.ledger_tag_graphics_footprint + info.ledger_tag_graphics_footprint_compressed;
    (read == mach2::kern_return::KERN_SUCCESS).then_some(bytes.max(0) as u64)
}

#[cfg(not(target_vendor = "apple"))]
fn graphics() -> Option<u64> {
    None
}

/// What the process's heap holds as its allocator counts it: every malloc zone's, a Mac's and an iPhone's.
#[cfg(target_vendor = "apple")]
fn heap() -> Option<u64> {
    let mut stats = libc::malloc_statistics_t { blocks_in_use: 0, size_in_use: 0, max_size_in_use: 0, size_allocated: 0 };
    // Safety: no zone is every zone's, and the statistics are filled in.
    unsafe { libc::malloc_zone_statistics(std::ptr::null_mut(), &mut stats) };
    Some(stats.size_in_use as u64)
}

#[cfg(not(target_vendor = "apple"))]
fn heap() -> Option<u64> {
    None
}
