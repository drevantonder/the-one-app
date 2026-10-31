//! `update` and `release`: the tip of main, tested and built on your own devices, and run on every one of them.
//! What runs anywhere is only ever what's on main.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use one::brand::{CLI, NAME, REPO, TARGET};
use serde_json::{Value, json};

use crate::{get, post};

/// Platforms the installer knows. A release has each one some device here can build.
const PLATFORMS: [&str; 3] = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "aarch64-apple-darwin"];

/// Builds the tip of main and has every device run it; with a tag, also signs it off and publishes it on GitHub.
pub fn run(tag: Option<&str>, space: Option<&str>) -> Result<()> {
    ensure!(!REPO.is_empty(), "this brand names no repo, so it has nothing to update from");
    ensure!(cfg!(target_os = "linux"), "run this on one of your Linux devices");
    if let Some(tag) = tag {
        ensure!(version_tag(tag), "a release is named like v1.2.3");
        ensure!(quiet(Command::new("gh").args(["auth", "status"])), "sign in to GitHub first: gh auth login");
    }
    let src = checkout()?;
    let commit = git(&src, &["rev-parse", "HEAD"])?;
    let short = &commit[..12];
    if let Some(tag) = tag {
        ensure!(!quiet(Command::new("git").current_dir(&src).args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])), "{tag} already exists");
    }
    println!("Testing {short}");
    step(Command::new(cargo()).current_dir(&src).arg("test"))?;
    if tag.is_some() {
        step(Command::new("gh").current_dir(&src).args(["signoff", "create", "--commit", &commit]))
            .context("sign off (gh extension install basecamp/gh-signoff)")?;
    }

    let space = find_space(&get("/v1/state")?, space)?;
    // The Macs build their app themselves, which takes longest, so they start first.
    let macs: Vec<Value> = devices(&space).into_iter().filter(|d| d["target"].as_str().is_some_and(|t| t.ends_with("-apple-darwin"))).collect();
    let mut started = Vec::new();
    for mac in &macs {
        let name = mac["name"].as_str().unwrap_or("");
        if mac["online"] != true {
            println!("  {name}: offline; run this again when it's back");
            continue;
        }
        match shell(&space, mac, &mac_job(&commit, tag.is_some()), 60) {
            Ok(r) if r["exit"] == 0 => started.push((mac.clone(), Instant::now())),
            Ok(r) => println!("  {name}: couldn't start its build: {}", r["stderr"].as_str().unwrap_or("").trim()),
            Err(err) => println!("  {name}: couldn't start its build: {err:#}"),
        }
    }

    println!("Building {short}");
    step(Command::new(cargo()).current_dir(&src).args(["build", "--release"]))?;
    let binary = src.join("target/release").join(CLI);
    crate::publish(&binary, space["id"].as_str().map(str::to_string), &Default::default())?;

    let started_at = Instant::now();
    let linux: Vec<Value> = devices(&space).into_iter().filter(|d| d["target"] == TARGET).collect();
    let mut failed = await_builds(&space, &linux, short, started_at, Duration::from_secs(180));

    let mut assets = Vec::new();
    if let Some(tag) = tag {
        let dir = cache().join("release").join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let name = format!("{CLI}-{TARGET}");
        std::fs::copy(&binary, dir.join(&name))?;
        assets.push(name);
    }
    for (mac, since) in &started {
        let name = mac["name"].as_str().unwrap_or("");
        match await_mac(&space, mac) {
            Ok(made) => {
                failed |= await_builds(&space, std::slice::from_ref(mac), short, *since, Duration::from_secs(120));
                if let Some(tag) = tag {
                    for (hash, asset) in made {
                        let bytes = blob(&hash).with_context(|| format!("get {asset} from {name}"))?;
                        std::fs::write(cache().join("release").join(tag).join(&asset), bytes)?;
                        assets.push(asset);
                    }
                }
            }
            Err(err) => {
                failed = true;
                println!("  {name}: {err:#}");
            }
        }
    }

    let Some(tag) = tag else {
        ensure!(!failed, "not every device runs {short}");
        return Ok(());
    };
    for p in PLATFORMS {
        if !assets.iter().any(|a| a.ends_with(p)) && !(p.ends_with("-apple-darwin") && assets.iter().any(|a| a == &app_zip())) {
            println!("No device here builds {p}, so {tag} has nothing for it.");
        }
    }
    let dir = cache().join("release").join(tag);
    assets.sort();
    let sums = Command::new("sha256sum").current_dir(&dir).args(&assets).output().context("sha256sum")?;
    ensure!(sums.status.success(), "sha256sum failed");
    std::fs::write(dir.join("checksums.txt"), sums.stdout)?;
    assets.push("checksums.txt".into());
    println!("Publishing {tag} on GitHub");
    let mut gh = Command::new("gh");
    gh.current_dir(&dir).args(["release", "create", tag, "--repo", REPO, "--target", &commit, "--generate-notes"]).args(&assets);
    step(&mut gh)?;
    ensure!(!failed, "released {tag}, but not every device runs it");
    Ok(())
}

/// `v1.2.3`, or `v1.2.3-rc.1`.
fn version_tag(tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix('v') else { return false };
    let core = rest.split_once('-').map_or(rest, |(core, pre)| if pre.is_empty() { "" } else { core });
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// What the Mac app is published as: the brand's name, without spaces, which GitHub won't keep in a file name.
fn app_zip() -> String {
    format!("{}-macos.zip", NAME.replace(' ', ""))
}

fn cache() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
    base.join(CLI)
}

/// A clean clone of the repo, kept in the cache, at the tip of main on GitHub. Build output stays, so builds are quick.
fn checkout() -> Result<PathBuf> {
    let dir = cache().join("src");
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(cache())?;
        step(Command::new("git").args(["clone", "-q", &format!("https://github.com/{REPO}.git")]).arg(&dir))?;
    }
    for args in [&["fetch", "-q", "--tags", "--force", "origin"][..], &["checkout", "-q", "--detach", "--force", "origin/main"], &["clean", "-qfdx", "-e", "/target"]] {
        step(Command::new("git").current_dir(&dir).args(args))?;
    }
    Ok(dir)
}

/// Cargo, also where rustup puts it, for a shell that doesn't have it on its PATH.
fn cargo() -> PathBuf {
    let rustup = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cargo/bin/cargo");
    if quiet(Command::new("cargo").arg("--version")) || !rustup.exists() { "cargo".into() } else { rustup }
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    ensure!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Runs a command, showing its output.
fn step(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("run {:?}", cmd.get_program()))?;
    ensure!(status.success(), "{:?} {} failed", cmd.get_program(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    Ok(())
}

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

fn find_space(state: &Value, name: Option<&str>) -> Result<Value> {
    let spaces: Vec<Value> = state["spaces"].as_array().cloned().unwrap_or_default();
    let found: Vec<Value> = spaces.into_iter().filter(|s| name.is_none_or(|n| s["name"] == n || s["id"].as_str().is_some_and(|id| id.starts_with(n)))).collect();
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => bail!("no such space"),
        _ => bail!("this device is in several spaces: say which with --space"),
    }
}

/// The space's devices that keep a log, as this device sees them now: none while its node restarts.
fn devices(space: &Value) -> Vec<Value> {
    let state = get("/v1/state").unwrap_or_default();
    let space = state["spaces"].as_array().into_iter().flatten().find(|s| s["id"] == space["id"]);
    space.and_then(|s| s["devices"].as_array()).into_iter().flatten().filter(|d| d["client"] != true).cloned().collect()
}

/// Waits for each of `wanted` to say it runs `commit`, and reports each. True if any didn't.
fn await_builds(space: &Value, wanted: &[Value], commit: &str, since: Instant, limit: Duration) -> bool {
    let id = |d: &Value| d["device"].as_str().unwrap_or("").to_string();
    let mut waiting: Vec<(String, String, String)> = wanted.iter().map(|d| (id(d), d["name"].as_str().unwrap_or("").into(), d["build"].as_str().unwrap_or("?").into())).collect();
    let deadline = Instant::now() + limit;
    loop {
        let now = devices(space);
        waiting.retain(|(device, name, was)| {
            let Some(d) = now.iter().find(|d| d["device"] == device.as_str()) else { return true };
            if d["build"] == commit {
                if was == commit {
                    println!("  {name}: already on {commit}");
                } else {
                    println!("  {name}: {was} → {commit} ({}s)", since.elapsed().as_secs());
                }
                return false;
            }
            if d["online"] != true {
                println!("  {name}: offline; it updates itself when it's back");
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

fn shell(space: &Value, device: &Value, command: &str, timeout: u64) -> Result<Value> {
    let r = post("/v1/call", json!({ "space": space["id"], "device": device["device"], "app": "shell", "op": "run", "input": { "command": command, "timeout": timeout } }))?;
    Ok(r["ok"].clone())
}

/// Shell for a Mac: start a job in its logged-in session (codesign needs the keychain, and the job outlives the node,
/// which the new app replaces) that builds `commit`'s app in a clean clone and installs it. For a release it also
/// leaves the app, zipped, and the node binary as blobs, listed in assets.txt.
fn mac_job(commit: &str, assets: bool) -> String {
    let assets = if assets {
        format!(
            r#"a="$c/assets"; rm -rf "$a"; mkdir "$a"
cp target/release/{CLI} "$a/{CLI}-$(rustc -vV | sed -n 's/^host: //p')"
ditto -c -k --keepParent "$HOME/Applications/{NAME}.app" "$a/{zip}"
for f in "$a"/*; do
	n=0; until h=$(target/release/{CLI} blob put "$f"); do n=$((n + 1)); [ $n -lt 30 ]; sleep 2; done
	echo "$h $(basename "$f")" >> "$c/assets.txt"
done"#,
            zip = app_zip()
        )
    } else {
        String::new()
    };
    format!(
        r#"set -e
c="$HOME/Library/Caches/{CLI}"; label=app.{CLI}.update
mkdir -p "$c"; cd "$c"; rm -f update.status assets.txt
cat > update.sh <<'JOB'
c="$HOME/Library/Caches/{CLI}"
exec > "$c/update.log" 2>&1
trap 'echo $? > "$c/update.status"' EXIT
set -e
[ -d "$c/src/.git" ] || git clone -q https://github.com/{REPO}.git "$c/src"
cd "$c/src"
git fetch -q --tags --force origin
git checkout -q --detach --force {commit}
git clean -qfdx -e /target -e /apple/build
apple/build.sh
{assets}
JOB
cat > update.plist <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>Label</key><string>$label</string><key>RunAtLoad</key><true/>
<key>ProgramArguments</key><array><string>${{SHELL:-/bin/zsh}}</string><string>-l</string><string>$c/update.sh</string></array>
</dict></plist>
PLIST
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" update.plist"#
    )
}

/// Waits for a Mac's job to finish; on success, the assets it left as `(blob, name)`.
fn await_mac(space: &Value, mac: &Value) -> Result<Vec<(String, String)>> {
    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    let poll = format!(r#"c="$HOME/Library/Caches/{CLI}"; cat "$c/update.status" 2>/dev/null"#);
    // The Mac's node restarts on the new app part way through, and doesn't answer for a moment.
    let status = loop {
        ensure!(Instant::now() < deadline, "its build took more than 30 minutes");
        std::thread::sleep(Duration::from_secs(5));
        if let Ok(r) = shell(space, mac, &poll, 30)
            && let Some(code) = r["stdout"].as_str().and_then(|s| s.trim().parse::<i32>().ok())
        {
            break code;
        }
    };
    let report = shell(space, mac, &format!(r#"c="$HOME/Library/Caches/{CLI}"; launchctl bootout "gui/$(id -u)/app.{CLI}.update" 2>/dev/null; cat "$c/assets.txt" 2>/dev/null; echo ---; tail -n 25 "$c/update.log""#), 30)?;
    let out = report["stdout"].as_str().unwrap_or("");
    let (made, log) = out.split_once("---\n").unwrap_or(("", out));
    if status != 0 {
        bail!("its build failed:\n{log}");
    }
    Ok(made.lines().filter_map(|l| l.split_once(' ')).map(|(h, n)| (h.to_string(), n.to_string())).collect())
}

fn blob(hash: &str) -> Result<Vec<u8>> {
    let mut resp = ureq::get(format!("http://{}/v1/blobs/{hash}", crate::addr())).call()?;
    Ok(resp.body_mut().with_config().limit(1 << 30).read_to_vec()?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_mac_job_is_shell() {
        for assets in [false, true] {
            let job = super::mac_job("0123abcd", assets);
            let inner = job.split_once("<<'JOB'\n").unwrap().1.split_once("\nJOB\n").unwrap().0;
            for script in [job.as_str(), inner] {
                let ok = std::process::Command::new("sh").args(["-n", "-c", script]).status().unwrap().success();
                assert!(ok, "{script}");
            }
        }
    }

    #[test]
    fn version_tags() {
        for good in ["v0.2.0", "v10.0.1", "v1.2.3-rc.1"] {
            assert!(super::version_tag(good), "{good}");
        }
        for bad in ["0.2.0", "v0.2", "v1.2.3-", "v1.x.3", "latest"] {
            assert!(!super::version_tag(bad), "{bad}");
        }
    }
}
