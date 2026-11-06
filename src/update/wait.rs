//! `status --until`: waiting until devices run a commit (or a later build) and their services answer.

use super::*;

/// This user's own devices, as this one sees them now: none while its node restarts.
pub(super) fn devices() -> Vec<Value> {
    let state = crate::node().get("/v1/state").unwrap_or_default();
    let me = state["network"]["user"].clone();
    state["network"]["devices"].as_array().into_iter().flatten().filter(|d| d["user"] == me).cloned().collect()
}

/// Waits for each of `wanted` to say it runs `commit`, and reports each. True if any didn't.
pub(super) fn await_builds(wanted: &[Value], commit: &str, since: Instant, limit: Duration) -> bool {
    let id = |d: &Value| d["key"].as_str().unwrap_or("").to_string();
    let name = |d: &Value| d["name"].as_str().unwrap_or("").to_string();
    // Only devices offline from the start count as offline: this device's own node restarting loses its sessions a moment.
    let (online, offline): (Vec<&Value>, Vec<&Value>) = wanted.iter().partition(|d| d["online"] == true);
    for d in offline {
        println!("  {}: offline; it updates itself when it's back", name(d));
    }
    let mut waiting: Vec<(String, String, String)> = online.into_iter().map(|d| (id(d), name(d), d["build"].as_str().unwrap_or("?").into())).collect();
    let deadline = Instant::now() + limit;
    loop {
        let now = devices();
        waiting.retain(|(device, name, was)| {
            let Some(d) = now.iter().find(|d| d["key"] == device.as_str()) else { return true };
            if d["build"] == commit {
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
            for (_, name, was) in &waiting {
                println!("  {name}: still on {was}");
            }
            return true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// What an update says last, when the Macs are still building in the background.
pub(super) fn says_wait(short: &str) {
    println!("The Macs build it in the background: {CLI} status --until {short} waits for them, and for every device's services.");
}

/// `status --until <commit> [--device <name>]… [--timeout <time>]`, with the words after `status`.
pub fn wait(args: &[String]) -> Result<()> {
    let (mut commit, mut names, mut limit) = (None, Vec::new(), Duration::from_secs(10 * 60));
    let mut words = args.iter().skip_while(|a| *a != "status").skip(1);
    while let Some(flag) = words.next() {
        let typed = |m: String| anyhow::Error::from(one::cli::Failed::Typed(m));
        let value = words.next().filter(|v| !v.starts_with("--")).ok_or_else(|| typed(format!("{flag} takes a value: see {CLI} status --help")))?;
        match flag.as_str() {
            "--until" => commit = Some(value.to_lowercase()),
            "--device" => names.extend(value.split(',').map(str::to_string)),
            "--timeout" => {
                let (n, unit) = value.split_at(value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len()));
                let n: u64 = n.parse().map_err(|_| typed("--timeout is like 90s, 10m or 1h".into()))?;
                limit = Duration::from_secs(n * match unit { "" | "s" => 1, "m" => 60, "h" => 3600, _ => return Err(typed("--timeout is like 90s, 10m or 1h".into())) });
            }
            other => return Err(typed(format!("{other} isn't an input of status --until: see {CLI} status --help"))),
        }
    }
    let commit = commit.unwrap_or_default();
    ensure!(commit.len() >= 7 && commit.chars().all(|c| c.is_ascii_hexdigit()), "give the commit: {CLI} status --until $(git rev-parse HEAD)");
    wait_until(&commit, &names, limit)
}

/// Waits until each of the user's devices (or those named) runs `commit`, or a build with it in, and its services
/// answer, or a Mac's build of it fails; says what each did, and fails unless all made it. Shell on a Mac says how its
/// builds went, which `follow` set up; a device that is off or restarting is waited for like the rest.
pub(super) fn wait_until(commit: &str, names: &[String], limit: Duration) -> Result<()> {
    let short = &commit[..commit.len().min(12)];
    // Says so when no node answers, which later polls (a node restarting on the new build) never do.
    crate::node().get("/v1/state")?;
    let all: Vec<Value> = devices();
    ensure!(!all.is_empty(), "no devices are listed here yet");
    for n in names {
        ensure!(all.iter().any(|d| d["name"] == n.as_str()), "no device named {n}: {}", all.iter().filter_map(|d| d["name"].as_str()).collect::<Vec<_>>().join(", "));
    }
    let mut open: Vec<(String, String, String)> =
        all.iter().filter(|d| names.is_empty() || names.iter().any(|n| d["name"] == n.as_str())).map(|d| (d["key"].as_str().unwrap_or("").into(), d["name"].as_str().unwrap_or("").into(), d["build"].as_str().unwrap_or("?").into())).collect();
    let (started, deadline) = (Instant::now(), Instant::now() + limit);
    let (mut fetched, mut reasons, mut asked, mut phones) = (std::collections::HashSet::new(), std::collections::HashMap::new(), std::collections::HashMap::<String, Instant>::new(), std::collections::HashMap::<String, String>::new());
    let mut failed = false;
    loop {
        let now = devices();
        open.retain(|(key, name, was)| {
            let Some(d) = now.iter().find(|d| d["key"] == key.as_str()) else { return true };
            let build = d["build"].as_str().unwrap_or("");
            let mac = d["target"].as_str().is_some_and(|t| Platform::is_mac(t));
            let mut said = None;
            let running = has(commit, build, &mut fetched);
            if mac && d["online"] == true && asked.get(key).is_none_or(|t| t.elapsed() > Duration::from_secs(6)) {
                asked.insert(key.clone(), Instant::now());
                said = mac_said(d);
                if let Some(phone) = said.as_ref().and_then(|s| s.phone.clone()) {
                    phones.insert(key.clone(), phone);
                }
            }
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
                    if let Some(phone) = phones.get(key) {
                        println!("    {phone}");
                    }
                    return false;
                }
                reasons.insert(key.clone(), format!("runs {build}, but its services aren't all answering: {}", troubled.join("; ")));
                return true;
            }
            if let Some(s) = said.as_ref() {
                if let Some(log) = &s.failed_log {
                    failed = true;
                    println!("  {name}: its build of {} failed: the end of its log, ~/Library/Caches/{CLI}/update.log on {name}, is", &s.tried[..s.tried.len().min(12)]);
                    for line in log.lines() {
                        println!("      {line}");
                    }
                    if let Some(phone) = phones.get(key) {
                        println!("    {phone}");
                    }
                    return false;
                }
                reasons.insert(key.clone(), s.waiting_for(commit, short, build, &mut fetched));
            } else if d["online"] != true {
                reasons.insert(key.clone(), format!("offline, on {build}; it updates itself when it's back"));
            } else if !mac {
                reasons.insert(key.clone(), format!("still on {build}"));
            }
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
