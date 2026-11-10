use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::{Guest, Load, Space};

/// CPU and GPU are sampled this often, and memory, disk and containers every `SLOW` samples.
const EVERY: Duration = Duration::from_secs(5);
const SLOW: usize = 6;
/// Two minutes of samples.
const KEEP: usize = 24;

static LOAD: OnceLock<Mutex<Load>> = OnceLock::new();

/// This machine's load, as last sampled. The first call starts sampling.
pub fn current() -> Load {
    static STARTED: std::sync::Once = std::sync::Once::new();
    let load = LOAD.get_or_init(|| Mutex::new(Load { container: in_container(), kind: kind().into(), ..Load::default() }));
    STARTED.call_once(|| drop(std::thread::Builder::new().name("load".into()).spawn(sample)));
    load.lock().unwrap().clone()
}

fn sample() {
    let mut cpu = Cpu::default();
    for n in 0.. {
        let container = in_container();
        let slow = (n % SLOW == 0).then(|| (memory(), disk(), if container { Vec::new() } else { guests() }));
        let busy = cpu.busy();
        let gpu = if container { None } else { gpu() };
        if let Some(lock) = LOAD.get() {
            let mut l = lock.lock().unwrap();
            if let Some((memory, disk, containers)) = slow {
                (l.memory, l.disk, l.containers) = (memory, disk, containers);
            }
            push(&mut l.cpu, busy);
            push(&mut l.gpu, gpu);
            l.container = container;
        }
        std::thread::sleep(EVERY);
    }
}

fn push(to: &mut Vec<u8>, sample: Option<u8>) {
    if let Some(s) = sample {
        to.push(s.min(100));
        let extra = to.len().saturating_sub(KEEP);
        to.drain(..extra);
    }
}

/// CPU time, busy and in all, since boot; `busy` gives the percent busy since it was last asked.
#[derive(Default)]
struct Cpu {
    last: Option<(u64, u64)>,
}

impl Cpu {
    fn busy(&mut self) -> Option<u8> {
        let (busy, total) = cpu_ticks()?;
        let last = self.last.replace((busy, total));
        let (b, t) = last?;
        (total > t).then(|| ((busy.saturating_sub(b)) * 100 / (total - t)) as u8)
    }
}

/// What this machine is: a `laptop` if it has a battery (it sleeps), `cloud` if it's a server rented from a cloud by the
/// hour, else `always_on`. Any OS: what's asked is the machine's firmware and power, never which OS it runs.
fn kind() -> &'static str {
    if battery() {
        "laptop"
    } else if CLOUDS.iter().any(|c| firmware().contains(c)) {
        "cloud"
    } else {
        "always_on"
    }
}

/// What a rented cloud server's firmware says (Azure's is its chassis asset tag).
const CLOUDS: &[&str] = &["Amazon EC2", "Google Compute Engine", "DigitalOcean", "Hetzner", "Vultr", "Linode", "Akamai", "OracleCloud", "Scaleway", "Alibaba Cloud", "7783-7084-3265-9085-8269-3286-77"];

#[cfg(target_os = "linux")]
fn battery() -> bool {
    std::fs::read_dir("/sys/class/power_supply").into_iter().flatten().flatten().any(|d| {
        let read = |f: &str| std::fs::read_to_string(d.path().join(f)).unwrap_or_default();
        // A mouse's or keyboard's battery is a "Device"'s.
        read("type").trim() == "Battery" && read("scope").trim() != "Device"
    })
}

#[cfg(target_os = "linux")]
fn firmware() -> String {
    ["sys_vendor", "product_name", "bios_vendor", "chassis_asset_tag"].iter().map(|f| std::fs::read_to_string(format!("/sys/class/dmi/id/{f}")).unwrap_or_default()).collect::<Vec<_>>().join("\n")
}

#[cfg(target_os = "macos")]
fn battery() -> bool {
    run("/usr/bin/pmset", &["-g", "batt"]).is_some_and(|o| o.contains("InternalBattery"))
}

#[cfg(target_os = "macos")]
fn firmware() -> String {
    String::new()
}

#[cfg(windows)]
fn battery() -> bool {
    run("powershell", &["-NoProfile", "-Command", "@(Get-CimInstance Win32_Battery).Count"]).is_some_and(|o| o.trim() != "0")
}

#[cfg(windows)]
fn firmware() -> String {
    run("powershell", &["-NoProfile", "-Command", "$s = Get-CimInstance Win32_ComputerSystem; $e = Get-CimInstance Win32_SystemEnclosure; $b = Get-CimInstance Win32_BIOS; $s.Manufacturer; $s.Model; $b.Manufacturer; $e.SMBIOSAssetTag"]).unwrap_or_default()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn battery() -> bool {
    false
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn firmware() -> String {
    String::new()
}

fn in_container() -> bool {
    cfg!(target_os = "linux") && ["/run/systemd/container", "/run/.containerenv", "/.dockerenv"].iter().any(|p| std::path::Path::new(p).exists())
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(target_os = "linux")]
fn find(paths: &[&'static str]) -> Option<&'static str> {
    paths.iter().copied().find(|p| std::path::Path::new(p).exists())
}

fn disk() -> Option<Space> {
    let path = if cfg!(target_os = "macos") { "/System/Volumes/Data" } else if cfg!(windows) { "C:\\" } else { "/" };
    let (used, free) = crate::os::disk_space(std::path::Path::new(path))?;
    Some(Space { used, total: used + free })
}

#[cfg(target_os = "linux")]
fn memory() -> Option<Space> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "macos")]
fn memory() -> Option<Space> {
    static TOTAL: OnceLock<Option<u64>> = OnceLock::new();
    let total = (*TOTAL.get_or_init(|| run("/usr/sbin/sysctl", &["-n", "hw.memsize"])?.trim().parse().ok()))?;
    parse_vm_stat(&run("/usr/bin/vm_stat", &[])?, total)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn memory() -> Option<Space> {
    None
}

#[cfg(target_os = "linux")]
fn cpu_ticks() -> Option<(u64, u64)> {
    parse_proc_stat(std::fs::read_to_string("/proc/stat").ok()?.lines().next()?)
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn cpu_ticks() -> Option<(u64, u64)> {
    let mut info: libc::host_cpu_load_info = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    let host = unsafe { libc::mach_host_self() };
    let r = unsafe { libc::host_statistics(host, libc::HOST_CPU_LOAD_INFO, &mut info as *mut _ as *mut i32, &mut count) };
    if r != 0 {
        return None;
    }
    let t = info.cpu_ticks.map(u64::from);
    let total: u64 = t.iter().sum();
    Some((total - t[libc::CPU_STATE_IDLE as usize], total))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn cpu_ticks() -> Option<(u64, u64)> {
    None
}

#[cfg(target_os = "linux")]
fn gpu() -> Option<u8> {
    let mut busiest: Option<u8> = None;
    let mut see = |p: u8| busiest = Some(busiest.map_or(p, |b| b.max(p)));
    for card in std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten() {
        if let Ok(s) = std::fs::read_to_string(card.path().join("device/gpu_busy_percent")) {
            s.trim().parse().map(&mut see).ok();
        }
    }
    if let Some(smi) = find(&["/usr/bin/nvidia-smi", "/usr/local/bin/nvidia-smi"]) {
        for l in run(smi, &["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"]).unwrap_or_default().lines() {
            l.trim().parse().map(&mut see).ok();
        }
    }
    busiest
}

#[cfg(target_os = "macos")]
fn gpu() -> Option<u8> {
    parse_ioreg(&run("/usr/sbin/ioreg", &["-r", "-d", "1", "-c", "IOAccelerator"])?).into_iter().max()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn gpu() -> Option<u8> {
    None
}

/// The containers running here: Incus or LXD system containers, from their cgroups, and this user's podman ones.
#[cfg(target_os = "linux")]
fn guests() -> Vec<Guest> {
    let mut out = Vec::new();
    for dir in std::fs::read_dir("/sys/fs/cgroup").into_iter().flatten().flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        let Some(name) = name.strip_prefix("lxc.payload.") else { continue };
        let read = |f: &str| std::fs::read_to_string(dir.path().join(f)).unwrap_or_default();
        let memory = parse_cgroup_memory(&read("memory.current"), &read("memory.stat"), &read("memory.max"));
        out.push(Guest { name: name.to_string(), kind: "system".into(), memory });
    }
    if let Some(podman) = find(&["/usr/bin/podman", "/usr/local/bin/podman"]) {
        out.extend(parse_podman_stats(&run(podman, &["stats", "--no-stream", "--format", "json"]).unwrap_or_default()));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(not(target_os = "linux"))]
fn guests() -> Vec<Guest> {
    Vec::new()
}

/// /proc/meminfo: total minus available.
pub fn parse_meminfo(s: &str) -> Option<Space> {
    let kb = |key: &str| -> Option<u64> { s.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':')?.trim().strip_suffix("kB")?.trim().parse().ok()) };
    let (total, available) = (kb("MemTotal")?, kb("MemAvailable")?);
    (total > 0 && available <= total).then(|| Space { used: (total - available) * 1024, total: total * 1024 })
}

/// /proc/stat's first line, `cpu user nice system idle iowait irq softirq steal …`: busy and total ticks.
pub fn parse_proc_stat(line: &str) -> Option<(u64, u64)> {
    let mut f = line.split_whitespace();
    if f.next()? != "cpu" {
        return None;
    }
    let ticks: Vec<u64> = f.take(8).map(|v| v.parse().ok()).collect::<Option<_>>()?;
    let total: u64 = ticks.iter().sum();
    let idle = ticks.get(3).copied().unwrap_or(0) + ticks.get(4).copied().unwrap_or(0);
    (ticks.len() >= 4).then_some((total - idle, total))
}

/// `vm_stat`, as Activity Monitor counts "Memory Used": app memory (anonymous pages, less purgeable ones), wired and
/// compressed.
pub fn parse_vm_stat(s: &str, total: u64) -> Option<Space> {
    let page: u64 = s.lines().next()?.split("page size of ").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let pages = |key: &str| -> u64 { s.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':')?.trim().trim_end_matches('.').parse().ok()).unwrap_or(0) };
    let app = pages("Anonymous pages").saturating_sub(pages("Pages purgeable"));
    let used = (app + pages("Pages wired down") + pages("Pages occupied by compressor")) * page;
    Some(Space { used: used.min(total), total })
}

/// Every `"Device Utilization %"=N` in `ioreg -c IOAccelerator`.
pub fn parse_ioreg(s: &str) -> Vec<u8> {
    s.split("\"Device Utilization %\"=").skip(1).filter_map(|r| r.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).collect()
}

/// A cgroup's memory, less the files it caches; its limit, if it has one.
pub fn parse_cgroup_memory(current: &str, stat: &str, max: &str) -> Option<Space> {
    let current: u64 = current.trim().parse().ok()?;
    let stat = |key: &str| -> u64 { stat.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(' ')?.trim().parse().ok()).unwrap_or(0) };
    let used = current.saturating_sub(stat("active_file") + stat("inactive_file"));
    Some(Space { used, total: max.trim().parse().unwrap_or(0) })
}

/// `podman stats --no-stream --format json`: each container's name and memory ("29.93MB / 4.295GB").
pub fn parse_podman_stats(s: &str) -> Vec<Guest> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
    rows.iter()
        .filter_map(|r| {
            let name = r["name"].as_str()?.to_string();
            let (used, total) = r["mem_usage"].as_str().and_then(|m| m.split_once(" / ")).unzip();
            let memory = used.and_then(size).map(|used| Space { used, total: total.and_then(size).unwrap_or(0) });
            Some(Guest { name, kind: "podman".into(), memory })
        })
        .collect()
}

/// A size as podman prints it: `4.295GB`, `29.93MB`, `512kB`, `1.5GiB`, `0B`.
fn size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (n, unit) = s.split_at(split);
    let n: f64 = n.parse().ok()?;
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
    Some((n * scale) as u64)
}
