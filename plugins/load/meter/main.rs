//! Measures the machine it runs on: for each line it reads, it writes one line of JSON with the machine's
//! processors, its load over the last 1, 5 and 15 minutes, its memory and disk in use, how busy its CPU and GPU
//! were every 5 seconds over the last two minutes, the containers it hosts, and when this program started. It
//! stops when its input closes, which is how it's told that the service that started it has gone.
//!
//! A thread measures in the background, so a line is answered at once: CPU and GPU every 5 seconds, the rest every
//! 30. It's built with nothing but `rustc`, so what it reads of Linux and Mac is read by hand.

use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

extern "C" {
    /// The C library's load averages, on Linux and Mac.
    fn getloadavg(averages: *mut f64, count: i32) -> i32;
}

/// CPU and GPU are measured this often, and the rest every `SLOW` times.
const EVERY: Duration = Duration::from_secs(5);
const SLOW: usize = 6;
/// Two minutes of CPU and GPU.
const KEEP: usize = 24;

/// Bytes in use, of how many there are. A container with no limit of its own has a total of 0.
#[derive(Clone, Copy)]
struct Space {
    used: u64,
    total: u64,
}

#[derive(Clone)]
struct Container {
    name: String,
    kind: &'static str,
    memory: Option<Space>,
}

#[derive(Clone, Default)]
struct Measured {
    memory: Option<Space>,
    disk: Option<Space>,
    cpu: Vec<u8>,
    gpu: Vec<u8>,
    containers: Vec<Container>,
}

fn main() {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let processors = std::thread::available_parallelism().map_or(1, |processors| processors.get());
    // What's measured every 30 seconds is measured once first, so the first line has it.
    let containers = if in_container() { Vec::new() } else { containers() };
    let measured = Arc::new(Mutex::new(Measured { memory: memory(), disk: disk(), containers, ..Measured::default() }));
    let measuring = measured.clone();
    std::thread::spawn(move || measure(&measuring));
    let mut out = io::stdout();
    for asked in io::stdin().lock().lines() {
        if asked.is_err() {
            break;
        }
        let mut load = [0f64; 3];
        // SAFETY: `load` has room for the three averages asked for.
        unsafe { getloadavg(load.as_mut_ptr(), 3) };
        let [one, five, fifteen] = load;
        let now = measured.lock().unwrap().clone();
        let os = std::env::consts::OS;
        let (memory, disk, cpu, gpu) = (space(now.memory), space(now.disk), list(&now.cpu), list(&now.gpu));
        let containers: Vec<String> = now
            .containers
            .iter()
            .map(|c| format!("{{\"name\":{},\"kind\":\"{}\",\"memory\":{}}}", text(&c.name), c.kind, space(c.memory)))
            .collect();
        let containers = containers.join(",");
        let measured = format!(
            "{{\"processors\":{processors},\"load\":[{one:.2},{five:.2},{fifteen:.2}],\"since\":{since},\"os\":\"{os}\",\"memory\":{memory},\"disk\":{disk},\"cpu\":{cpu},\"gpu\":{gpu},\"containers\":[{containers}]}}\n"
        );
        if out.write_all(measured.as_bytes()).and_then(|()| out.flush()).is_err() {
            break;
        }
    }
}

/// Whether this machine is a container itself, whose GPU and containers are its host's.
fn in_container() -> bool {
    cfg!(target_os = "linux")
        && ["/run/systemd/container", "/run/.containerenv", "/.dockerenv"]
            .iter()
            .any(|p| std::path::Path::new(p).exists())
}

fn measure(measured: &Mutex<Measured>) {
    let in_container = in_container();
    let mut last = None;
    for n in 1.. {
        let slow = (n % SLOW == 0).then(|| (memory(), disk(), if in_container { Vec::new() } else { containers() }));
        let ticks = cpu_ticks();
        let cpu = match (last, ticks) {
            (Some((busy, total)), Some((now_busy, now_total))) if now_total > total => {
                Some(((now_busy - busy) * 100 / (now_total - total)) as u8)
            }
            _ => None,
        };
        last = ticks;
        let gpu = if in_container { None } else { gpu() };
        {
            let mut m = measured.lock().unwrap();
            if let Some((memory, disk, containers)) = slow {
                (m.memory, m.disk, m.containers) = (memory, disk, containers);
            }
            push(&mut m.cpu, cpu);
            push(&mut m.gpu, gpu);
        }
        std::thread::sleep(EVERY);
    }
}

fn push(to: &mut Vec<u8>, sample: Option<u8>) {
    if let Some(sample) = sample {
        to.push(sample.min(100));
        let extra = to.len().saturating_sub(KEEP);
        to.drain(..extra);
    }
}

fn space(space: Option<Space>) -> String {
    space.map_or("null".into(), |s| format!("{{\"used\":{},\"total\":{}}}", s.used, s.total))
}

fn list(samples: &[u8]) -> String {
    format!("[{}]", samples.iter().map(u8::to_string).collect::<Vec<_>>().join(","))
}

/// A string as JSON writes it.
fn text(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out + "\""
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The disk the user's files are on, as `df -Pk` says it, the same on Linux and Mac: in use, and in use or free.
/// What's kept back for the system counts as neither, as Finder and `df` have it.
fn disk() -> Option<Space> {
    let path = if cfg!(target_os = "macos") { "/System/Volumes/Data" } else { "/" };
    let said = run("df", &["-Pk", path])?;
    let fields: Vec<&str> = said.lines().nth(1)?.split_whitespace().collect();
    let capacity = fields.iter().position(|f| f.ends_with('%'))?;
    let kb = |at: usize| -> Option<u64> { fields.get(capacity.checked_sub(at)?)?.parse().ok() };
    let (used, free) = (kb(2)? * 1024, kb(1)? * 1024);
    Some(Space { used, total: used + free })
}

/// /proc/meminfo: total, less what's available.
#[cfg(target_os = "linux")]
fn memory() -> Option<Space> {
    let said = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = |key: &str| -> Option<u64> {
        said.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':')?.trim().strip_suffix("kB")?.trim().parse().ok())
    };
    let (total, available) = (kb("MemTotal")?, kb("MemAvailable")?);
    (available <= total).then(|| Space { used: (total - available) * 1024, total: total * 1024 })
}

/// `vm_stat`, as Activity Monitor counts "Memory Used": app memory (anonymous pages, less purgeable ones), wired
/// and compressed.
#[cfg(target_os = "macos")]
fn memory() -> Option<Space> {
    let total: u64 = run("/usr/sbin/sysctl", &["-n", "hw.memsize"])?.trim().parse().ok()?;
    let said = run("/usr/bin/vm_stat", &[])?;
    let page: u64 = said.lines().next()?.split("page size of ").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let pages = |key: &str| -> u64 {
        said.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':')?.trim().trim_end_matches('.').parse().ok())
            .unwrap_or(0)
    };
    let app = pages("Anonymous pages").saturating_sub(pages("Pages purgeable"));
    let used = (app + pages("Pages wired down") + pages("Pages occupied by compressor")) * page;
    Some(Space { used: used.min(total), total })
}

/// CPU time busy, and in all, since the machine started: /proc/stat's first line, `cpu user nice system idle
/// iowait irq softirq steal`.
#[cfg(target_os = "linux")]
fn cpu_ticks() -> Option<(u64, u64)> {
    let said = std::fs::read_to_string("/proc/stat").ok()?;
    let mut fields = said.lines().next()?.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let ticks: Vec<u64> = fields.take(8).map(|t| t.parse().ok()).collect::<Option<_>>()?;
    let total: u64 = ticks.iter().sum();
    let idle = ticks.get(3)? + ticks.get(4).unwrap_or(&0);
    Some((total - idle, total))
}

#[cfg(target_os = "macos")]
extern "C" {
    fn mach_host_self() -> u32;
    fn host_statistics(host: u32, flavor: i32, info: *mut u32, count: *mut u32) -> i32;
}

/// CPU time busy, and in all, since the machine started: the kernel's `host_cpu_load_info`, whose ticks are user,
/// system, idle and nice, in that order.
#[cfg(target_os = "macos")]
fn cpu_ticks() -> Option<(u64, u64)> {
    const HOST_CPU_LOAD_INFO: i32 = 3;
    let mut ticks = [0u32; 4];
    let mut count = 4u32;
    // SAFETY: `ticks` has room for the `count` integers host_cpu_load_info is.
    if unsafe { host_statistics(mach_host_self(), HOST_CPU_LOAD_INFO, ticks.as_mut_ptr(), &mut count) } != 0 {
        return None;
    }
    let total: u64 = ticks.iter().map(|&t| u64::from(t)).sum();
    Some((total - u64::from(ticks[2]), total))
}

/// The busiest GPU, in percent: AMD's and Intel's as the kernel says, NVIDIA's as nvidia-smi does.
#[cfg(target_os = "linux")]
fn gpu() -> Option<u8> {
    let mut busiest: Option<u8> = None;
    let mut seen = |busy: u8| busiest = Some(busiest.map_or(busy, |b| b.max(busy)));
    for card in std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten() {
        if let Ok(busy) = std::fs::read_to_string(card.path().join("device/gpu_busy_percent")) {
            busy.trim().parse().map(&mut seen).ok();
        }
    }
    for line in
        run("nvidia-smi", &["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"]).unwrap_or_default().lines()
    {
        line.trim().parse().map(&mut seen).ok();
    }
    busiest
}

/// Every `"Device Utilization %"=N` in `ioreg -c IOAccelerator`.
#[cfg(target_os = "macos")]
fn gpu() -> Option<u8> {
    let said = run("/usr/sbin/ioreg", &["-r", "-d", "1", "-c", "IOAccelerator"])?;
    said.split("\"Device Utilization %\"=")
        .skip(1)
        .filter_map(|r| r.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok())
        .max()
}

/// The containers running here: Incus or LXD system containers, from their cgroups, and this user's podman ones.
#[cfg(target_os = "linux")]
fn containers() -> Vec<Container> {
    let mut out = Vec::new();
    for dir in std::fs::read_dir("/sys/fs/cgroup").into_iter().flatten().flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        let Some(name) = name.strip_prefix("lxc.payload.") else { continue };
        let read = |file: &str| std::fs::read_to_string(dir.path().join(file)).unwrap_or_default();
        // Its memory, less the files it caches, and its limit if it has one.
        let memory = read("memory.current").trim().parse::<u64>().ok().map(|current| {
            let stat = read("memory.stat");
            let of = |key: &str| -> u64 {
                stat.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(' ')?.trim().parse().ok()).unwrap_or(0)
            };
            Space {
                used: current.saturating_sub(of("active_file") + of("inactive_file")),
                total: read("memory.max").trim().parse().unwrap_or(0),
            }
        });
        out.push(Container { name: name.to_string(), kind: "system", memory });
    }
    // Each line is a name and its memory, as podman writes sizes: "29.93MB / 4.295GB".
    for line in
        run("podman", &["stats", "--no-stream", "--format", "{{.Name}}\t{{.MemUsage}}"]).unwrap_or_default().lines()
    {
        let Some((name, memory)) = line.split_once('\t') else { continue };
        let memory = memory
            .split_once(" / ")
            .and_then(|(used, total)| Some(Space { used: size(used)?, total: size(total).unwrap_or(0) }));
        out.push(Container { name: name.to_string(), kind: "podman", memory });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(target_os = "macos")]
fn containers() -> Vec<Container> {
    Vec::new()
}

/// A size as podman writes it: `4.295GB`, `29.93MB`, `512kB`, `1.5GiB`, `0B`.
#[cfg(target_os = "linux")]
fn size(said: &str) -> Option<u64> {
    let said = said.trim();
    let (number, unit) = said.split_at(said.find(|c: char| !(c.is_ascii_digit() || c == '.'))?);
    let scale = match unit.to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1048576.0,
        "gib" => 1073741824.0,
        "tib" => 1099511627776.0,
        _ => return None,
    };
    Some((number.parse::<f64>().ok()? * scale) as u64)
}
