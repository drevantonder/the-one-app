//! Waiting until devices run a commit (or a later build) and their services answer: `status --until`.

use super::*;

/// This user's own devices, as this one sees them now: none while its node restarts.
pub(super) fn devices() -> Vec<Value> {
    let state = crate::node().get("/v1/state").unwrap_or_default();
    let me = state["network"]["user"].clone();
    state["network"]["devices"].as_array().into_iter().flatten().filter(|d| d["user"] == me).cloned().collect()
}

/// `status --until <commit> [--device <name>]… [--timeout <time>]`.
pub fn wait(g: &one::cli::Given) -> Result<()> {
    g.takes_only(&["until", "device", "timeout"])?;
    let names: Vec<String> = g.values("device").iter().flat_map(|d| d.split(',')).map(str::to_string).collect();
    let limit = limit(g, 10 * 60)?;
    let commit = g.flag("until").unwrap_or_default().to_lowercase();
    let (code, config) = named(&commit);
    ensure!(code.len() >= 7 && [code, config.unwrap_or("0")].iter().all(|c| c.chars().all(|c| c.is_ascii_hexdigit())), "give the commit, or a custom build's <commit>-<config>: {CLI}-dev status --until $(git rev-parse HEAD)");
    wait_until(&commit, &names, limit)
}

/// `--timeout`, or `default` seconds.
pub(super) fn limit(g: &one::cli::Given, default: u64) -> Result<Duration> {
    let typed = || anyhow::Error::from(one::cli::Failed::Typed("--timeout is like 90s, 10m or 1h".into()));
    let Some(value) = g.flag("timeout") else { return Ok(Duration::from_secs(default)) };
    let (n, unit) = value.split_at(value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len()));
    let n: u64 = n.parse().map_err(|_| typed())?;
    Ok(Duration::from_secs(n * match unit { "" | "s" => 1, "m" => 60, "h" => 3600, _ => return Err(typed()) }))
}

/// Waits until each of the user's devices online now (or those named, online or not) runs `commit`, or a build with it
/// in, and its services answer, or its update fails; says what each did, and fails unless all made it. One that isn't
/// named and is offline at the start isn't waited for: it's named, with how far behind it is, and updates itself when
/// it's back. Why one hasn't yet is what its own node or Mac job says (`updating`).
pub(super) fn wait_until(commit: &str, names: &[String], limit: Duration) -> Result<()> {
    keep_history()?;
    // Says so when no node answers, which later polls (a node restarting on the new build) never do.
    crate::node().get("/v1/state")?;
    let all: Vec<Value> = devices();
    ensure!(!all.is_empty(), "no devices are listed here yet");
    for n in names {
        ensure!(all.iter().any(|d| d["name"] == n.as_str()), "no device named {n}: {}", all.iter().filter_map(|d| d["name"].as_str()).collect::<Vec<_>>().join(", "));
    }
    let wanted: Vec<Value> = all.into_iter().filter(|d| names.is_empty() || names.iter().any(|n| d["name"] == n.as_str())).collect();
    wait_for(commit, &wanted, !names.is_empty(), limit)
}

/// A copy of the repo's commits, kept for asking which build holds which: wherever this is run from, and without touching
/// the checkout an update builds in.
pub(super) fn history() -> PathBuf {
    cache().join("history")
}

pub(super) fn git_history() -> Command {
    let mut git = Command::new("git");
    git.arg("-C").arg(history());
    git
}

/// `history()`, made if it isn't there and up to date with the repo's.
pub(super) fn keep_history() -> Result<()> {
    if !history().join("HEAD").is_file() {
        std::fs::create_dir_all(cache())?;
        let next = cache().join(format!("history.{}", std::process::id()));
        show(Command::new("git").args(["clone", "-q", "--bare", "--filter=blob:none", REPO]).arg(&next))?;
        // Another command may have made it first: either's the same.
        if std::fs::rename(&next, history()).is_err() {
            let _ = std::fs::remove_dir_all(&next);
        }
    }
    fetch_history();
    Ok(())
}

fn fetch_history() -> bool {
    succeeds(git_history().args(["remote", "set-url", "origin", REPO])) && succeeds(git_history().args(["fetch", "-q", "origin", "+refs/heads/*:refs/heads/*"]))
}

/// A build's name, or what's given for one: its commit on main, and a custom build's config commit.
fn named(build: &str) -> (&str, Option<&str>) {
    build.split_once('-').map_or((build, None), |(code, config)| (code, Some(config)))
}

/// What a device on `target` ends up running for `commit`: `commit`, unless what's published for its platform is older
/// and nothing between them reaches a device of that platform (a change to the Workers or the web client alone), so
/// none will run it. Then it's what's published, and the device needn't wait for a build that will never come.
fn goal(commit: &str, target: &str, record: Option<&Latest>, fetched: &mut std::collections::HashSet<String>) -> String {
    let Some(published) = record.and_then(|r| r.for_target(target)) else { return commit.to_string() };
    if has(commit, &published.commit, fetched) {
        return commit.to_string();
    }
    let (from, to) = (named(&published.commit).0, named(commit).0);
    let Ok(out) = git_history().args(["diff", "--name-only", "--no-renames", from, to]).output() else { return commit.to_string() };
    if !out.status.success() {
        return commit.to_string();
    }
    let parts = Parts::of(String::from_utf8_lossy(&out.stdout).lines());
    if if one::build::Platform::is_mac(target) { parts.mac } else { parts.node } { commit.to_string() } else { published.commit.clone() }
}

/// The signed record at the network's address, if it can be read now.
fn record() -> Option<Latest> {
    Latest::read(RELEASE_KEY, &one::latest::fetch(&key_address()).ok()??).ok()
}

/// Waits until each of `wanted` runs `commit`, or a build with it in, with its link back and its services answering,
/// or its update fails; says what each did, and fails unless all made it. `named` says whether one offline at the
/// start is waited for too (it was asked for by name) or only named, with how far behind it is (it updates itself when
/// it's back). Why one hasn't yet is what its own node says (`updating`). The one place that says a device took a build.
pub(super) fn wait_for(commit: &str, wanted: &[Value], named: bool, limit: Duration) -> Result<()> {
    let short = &commit[..commit.len().min(12)];
    let mut fetched = std::collections::HashSet::new();
    let latest = record();
    let goals: std::collections::HashMap<String, String> = wanted.iter().map(|d| (d["key"].as_str().unwrap_or("").to_string(), goal(commit, d["target"].as_str().unwrap_or(""), latest.as_ref(), &mut fetched))).collect();
    let goal_of = |d: &Value| goals.get(d["key"].as_str().unwrap_or("")).map_or(commit, |g| g.as_str());
    for d in wanted.iter().filter(|d| goal_of(d) != commit) {
        println!("  {}: no build of this kind has {short} in it: waiting for {} instead", d["name"].as_str().unwrap_or(""), goal_of(d));
    }
    let (waited, away): (Vec<&Value>, Vec<&Value>) = wanted.iter().partition(|d| named || d["online"] == true);
    // Said at once, so a caller cut off before the end still saw what it waited for and who wasn't there.
    for d in away {
        let build = d["build"].as_str().unwrap_or("?");
        if !has(goal_of(d), build, &mut fetched) {
            println!("  {}: offline, on {build}{}: not waiting for it, it updates itself when it's back; --device {} to wait", d["name"].as_str().unwrap_or(""), behind(goal_of(d), build), d["name"].as_str().unwrap_or(""));
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
    let mut offline: std::collections::HashSet<String> = waited.iter().filter(|d| d["online"] != true && !has(goal_of(d), d["build"].as_str().unwrap_or(""), &mut fetched)).map(|d| d["key"].as_str().unwrap_or("").to_string()).collect();
    for d in waited.iter().filter(|d| offline.contains(d["key"].as_str().unwrap_or(""))) {
        println!("  {}: offline, on {}{}: waiting for it to come back", d["name"].as_str().unwrap_or(""), d["build"].as_str().unwrap_or("?"), behind(goal_of(d), d["build"].as_str().unwrap_or("")));
    }
    // A node restarting onto a build is listed as on it a moment before its link is back, and what deploys through it
    // needs the link; one offline from the start isn't held to that.
    let linked: std::collections::HashSet<String> = waited.iter().filter(|d| d["online"] == true).map(|d| d["key"].as_str().unwrap_or("").to_string()).collect();
    let mut failed = false;
    loop {
        let now = devices();
        open.retain(|(key, name, was)| {
            let Some(d) = now.iter().find(|d| d["key"] == key.as_str()) else { return true };
            let build = d["build"].as_str().unwrap_or("");
            if d["online"] == true && offline.remove(key) {
                println!("  {name}: back online, on {build}");
            }
            let running = has(goal_of(d), build, &mut fetched);
            if running && d["online"] != true && linked.contains(key) {
                reasons.insert(key.clone(), format!("runs {build}, but its link isn't back"));
                return true;
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

/// How far `build` is behind `commit`, as said after its name: nothing if git can't count them
/// (a build it hasn't got).
fn behind(commit: &str, build: &str) -> String {
    let Ok(out) = git_history().args(["rev-list", "--count", &format!("{}..{}", named(build).0, named(commit).0)]).stderr(std::process::Stdio::null()).output() else { return String::new() };
    if !out.status.success() {
        return String::new();
    }
    format!(", {} commits behind", String::from_utf8_lossy(&out.stdout).trim())
}

/// Whether `build` (what a device runs) is `commit` or has it in: a later update sends main's tip, which holds it. Either
/// may name a custom build's config too (`<commit>-<config>`): the same commit must then have that config, while a later
/// commit has whatever config came with it. Asks `history()`, which fetches once for a build it hasn't got.
pub(super) fn has(commit: &str, build: &str, fetched: &mut std::collections::HashSet<String>) -> bool {
    if build.is_empty() {
        return false;
    }
    let (want, want_config) = named(commit);
    let (code, config) = named(build);
    if want.starts_with(code) || code.starts_with(want) {
        return want_config.is_none_or(|w| config.is_some_and(|c| c.starts_with(w) || w.starts_with(c)));
    }
    let ancestor = || git_history().args(["merge-base", "--is-ancestor", want, code]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().ok().and_then(|s| s.code());
    match ancestor() {
        Some(0) => true,
        Some(1) => false,
        _ if fetched.insert(code.to_string()) => {
            fetch_history();
            ancestor() == Some(0)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_custom_builds_name_says_its_config_too() {
        let mut none = std::collections::HashSet::new();
        assert!(has("abc1234567", "abc1234567-def00000", &mut none), "the commit alone is met by a custom build of it");
        assert!(has("abc1234567890abc1234567890abc1234567890", "abc123456789-def00000", &mut none), "as git says the commit, in full");
        assert!(has("abc123456789-def00000", "abc123456789-def00000", &mut none));
        assert!(!has("abc123456789-def00000", "abc123456789-999fffff", &mut none), "the same commit with another config isn't it");
    }
}
