//! `status --until`: waiting until devices run a commit (or a later build) and their services answer.

use super::*;

/// This user's own devices, as this one sees them now: none while its node restarts.
pub(super) fn devices() -> Vec<Value> {
    let state = crate::node().get("/v1/state").unwrap_or_default();
    let me = state["network"]["user"].clone();
    state["network"]["devices"].as_array().into_iter().flatten().filter(|d| d["user"] == me).cloned().collect()
}

/// Waits for each of `wanted` to say it runs `commit` and to be online again (a node restarting onto a build is listed
/// as on it a moment before its link is back, and what deploys through it needs the link), and reports each. True if any didn't.
pub(super) fn await_builds(wanted: &[Value], commit: &str, since: Instant, limit: Duration) -> bool {
    let id = |d: &Value| d["key"].as_str().unwrap_or("").to_string();
    let name = |d: &Value| d["name"].as_str().unwrap_or("").to_string();
    // Only devices offline from the start count as offline: this device's own node restarting loses its sessions a moment.
    let (online, offline): (Vec<&Value>, Vec<&Value>) = wanted.iter().partition(|d| d["online"] == true);
    for d in offline {
        println!("  {}: offline; it updates itself when it's back", name(d));
    }
    let mut waiting: Vec<(String, String, String)> = online.into_iter().map(|d| (id(d), name(d), d["build"].as_str().unwrap_or("?").into())).collect();
    let mut seen = std::collections::HashMap::<String, String>::new();
    let deadline = Instant::now() + limit;
    loop {
        let now = devices();
        waiting.retain(|(device, name, was)| {
            let Some(d) = now.iter().find(|d| d["key"] == device.as_str()) else { return true };
            if d["build"] == commit {
                if d["online"] != true {
                    seen.insert(device.clone(), format!("runs {commit}, but its link isn't back"));
                    return true;
                }
                if was == commit {
                    println!("  {name}: already on {commit}");
                } else {
                    println!("  {name}: {was} → {commit} ({}s)", since.elapsed().as_secs());
                }
                return false;
            }
            true
        });
        if waiting.is_empty() {
            return false;
        }
        if Instant::now() > deadline {
            for (device, name, was) in &waiting {
                println!("  {name}: {}", seen.get(device).cloned().unwrap_or_else(|| format!("still on {was}")));
            }
            return true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// `status --until <commit> [--device <name>]… [--timeout <time>]`.
pub fn wait(g: &one::cli::Given) -> Result<()> {
    g.takes_only(&["until", "device", "timeout"])?;
    let typed = |m: &str| anyhow::Error::from(one::cli::Failed::Typed(m.into()));
    let names: Vec<String> = g.values("device").iter().flat_map(|d| d.split(',')).map(str::to_string).collect();
    let limit = match g.flag("timeout") {
        None => Duration::from_secs(10 * 60),
        Some(value) => {
            let (n, unit) = value.split_at(value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len()));
            let n: u64 = n.parse().map_err(|_| typed("--timeout is like 90s, 10m or 1h"))?;
            Duration::from_secs(n * match unit { "" | "s" => 1, "m" => 60, "h" => 3600, _ => return Err(typed("--timeout is like 90s, 10m or 1h")) })
        }
    };
    let commit = g.flag("until").unwrap_or_default().to_lowercase();
    ensure!(commit.len() >= 7 && commit.chars().all(|c| c.is_ascii_hexdigit()), "give the commit: {CLI} status --until $(git rev-parse HEAD)");
    wait_until(&commit, &names, limit)
}

/// Waits until each of the user's devices online now (or those named, online or not) runs `commit`, or a build with it
/// in, and its services answer, or its update fails; says what each did, and fails unless all made it. One that isn't
/// named and is offline at the start isn't waited for: it's named, with how far behind it is, and updates itself when
/// it's back. Why one hasn't yet is what its own node or Mac job says (`updating`).
pub(super) fn wait_until(commit: &str, names: &[String], limit: Duration) -> Result<()> {
    let short = &commit[..commit.len().min(12)];
    // Says so when no node answers, which later polls (a node restarting on the new build) never do.
    crate::node().get("/v1/state")?;
    let all: Vec<Value> = devices();
    ensure!(!all.is_empty(), "no devices are listed here yet");
    for n in names {
        ensure!(all.iter().any(|d| d["name"] == n.as_str()), "no device named {n}: {}", all.iter().filter_map(|d| d["name"].as_str()).collect::<Vec<_>>().join(", "));
    }
    let mut fetched = std::collections::HashSet::new();
    let (waited, away): (Vec<&Value>, Vec<&Value>) = all.iter().partition(|d| if names.is_empty() { d["online"] == true } else { names.iter().any(|n| d["name"] == n.as_str()) });
    // Said at once, so a caller cut off before the end still saw what it waited for and who wasn't there.
    for d in away {
        let build = d["build"].as_str().unwrap_or("?");
        if !has(commit, build, &mut fetched) {
            println!("  {}: offline, on {build}{}: not waiting for it, it updates itself when it's back; --device {} to wait", d["name"].as_str().unwrap_or(""), behind(commit, build), d["name"].as_str().unwrap_or(""));
        }
    }
    let mut open: Vec<(String, String, String)> = waited.iter().map(|d| (d["key"].as_str().unwrap_or("").into(), d["name"].as_str().unwrap_or("").into(), d["build"].as_str().unwrap_or("?").into())).collect();
    if open.is_empty() {
        println!("No device to wait for: none is online.");
        return Ok(());
    }
    let (started, deadline) = (Instant::now(), Instant::now() + limit);
    let mut reasons = std::collections::HashMap::new();
    println!("Waiting up to {}s for {} to run {short} or a later build.", limit.as_secs(), open.iter().map(|(_, name, _)| name.as_str()).collect::<Vec<_>>().join(", "));
    let mut offline: std::collections::HashSet<String> = waited.iter().filter(|d| d["online"] != true && !has(commit, d["build"].as_str().unwrap_or(""), &mut fetched)).map(|d| d["key"].as_str().unwrap_or("").to_string()).collect();
    for d in waited.iter().filter(|d| offline.contains(d["key"].as_str().unwrap_or(""))) {
        println!("  {}: offline, on {}{}: waiting for it to come back", d["name"].as_str().unwrap_or(""), d["build"].as_str().unwrap_or("?"), behind(commit, d["build"].as_str().unwrap_or("")));
    }
    let mut failed = false;
    loop {
        let now = devices();
        open.retain(|(key, name, was)| {
            let Some(d) = now.iter().find(|d| d["key"] == key.as_str()) else { return true };
            let build = d["build"].as_str().unwrap_or("");
            if d["online"] == true && offline.remove(key) {
                println!("  {name}: back online, on {build}");
            }
            let running = has(commit, build, &mut fetched);
            if running {
                let troubled: Vec<String> = d["parts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|p| p["on"] == true)
                    .filter_map(|p| {
                        let why = p["problem"].as_str().map(str::to_string).or_else(|| p["doing"].as_str().map(|d| d.to_lowercase()))?;
                        Some(format!("{}: {}", p["name"].as_str()?, why.trim_end_matches('.')))
                    })
                    .collect();
                if troubled.is_empty() {
                    if was == build {
                        println!("  {name}: already running {build}");
                    } else {
                        println!("  {name}: {was} → {build} ({}s)", started.elapsed().as_secs());
                    }
                    return false;
                }
                reasons.insert(key.clone(), format!("runs {build}, but its services aren't all answering: {}", troubled.join("; ")));
                return true;
            }
            let said = d["updating"].as_str().unwrap_or("");
            if let Some(why) = said.strip_prefix("Failed: ") {
                failed = true;
                println!("  {name}: {why}");
                return false;
            }
            reasons.insert(key.clone(), if d["online"] != true { format!("offline, on {build}; it updates itself when it's back") } else if said.is_empty() { format!("still on {build}") } else { format!("on {build}: {said}") });
            true
        });
        if open.is_empty() {
            ensure!(!failed, "not every device runs {short}");
            return Ok(());
        }
        if Instant::now() > deadline {
            for (key, name, _) in &open {
                println!("  {name}: {}", reasons.get(key).map_or("not answering".to_string(), |r| r.clone()));
            }
            println!("Gave up waiting for {short} after {}s.", limit.as_secs());
            bail!("not every device runs {short}");
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// How far `build` is behind `commit`, in this folder's git, as said after its name: nothing if git can't count them
/// (a build it hasn't got).
fn behind(commit: &str, build: &str) -> String {
    let Ok(out) = Command::new("git").args(["rev-list", "--count", &format!("{build}..{commit}")]).stderr(std::process::Stdio::null()).output() else { return String::new() };
    if !out.status.success() {
        return String::new();
    }
    format!(", {} commits behind", String::from_utf8_lossy(&out.stdout).trim())
}

/// Whether `build` (what a device runs) is `commit` or has it in: a later update sends main's tip, which holds it.
/// Asks this folder's git, which fetches once for a build it hasn't got.
pub(super) fn has(commit: &str, build: &str, fetched: &mut std::collections::HashSet<String>) -> bool {
    if build.is_empty() {
        return false;
    }
    if commit.starts_with(build) || build.starts_with(commit) {
        return true;
    }
    let ancestor = || Command::new("git").args(["merge-base", "--is-ancestor", commit, build]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().ok().and_then(|s| s.code());
    match ancestor() {
        Some(0) => true,
        Some(1) => false,
        _ if fetched.insert(build.to_string()) => {
            succeeds(Command::new("git").args(["fetch", "-q", "origin"]));
            ancestor() == Some(0)
        }
        _ => false,
    }
}
