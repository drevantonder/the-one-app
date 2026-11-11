//! The Mac app, and the iPhone app on a simulator of its own, built and run over ssh on the Mac.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ::custom_build::config::{CLI, NAME};

use super::super::devices::{approve_code, didnt_sign_in, shell_quote};
use super::super::network::spawn;

/// The Mac app (native-apps), or with `iphone` the iPhone app on a simulator, as this checkout builds it, on the Mac `MAC` names to ssh (`mac` if none), beside the
/// user's own app there: a throwaway's build (its own id, folder and Keychain items), in the Mac's logged-in session,
/// reaching the throwaway network at its address through ports ssh forwards there; the Mac app the window of a computer of
/// its own there, which sets it up as a real Mac's does, from what this publishes at the throwaway network's address of
/// this checkout's build for a Mac (its program and helper apps, and the window, for `<cli> update`). Signed in as the throwaway network's
/// first user as linux() is: its Sign In pressed, the sign-in approved as them and opened there, its code approved.
/// What it keeps stays on the Mac, so a new build is still signed in; it's started again each time.
pub(crate) fn apple(dir: &Path, checkout: &Path, settings: &oned_cloudflare::Settings, iphone: bool, started: Instant) -> Result<()> {
    let address = settings.address.as_str();
    let host = std::env::var("MAC").unwrap_or_else(|_| "mac".into());
    let ssh = |command: &str| -> Result<String> {
        let out = std::process::Command::new("ssh").args(["-o", "BatchMode=yes", &host, command]).output()?;
        anyhow::ensure!(out.status.success(), "on {host}: {command}: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    // This checkout as it is now, tracked and new files, in the Mac's cache, where its builds stay between runs.
    let there = format!("Library/Caches/{CLI}/throwaway-{}", checkout.file_name().unwrap_or_default().to_string_lossy());
    // What `stop` takes down there: this host's, and the copy's folder, which it deletes.
    std::fs::write(dir.join(if iphone { "iphone" } else { "mac" }), format!("{host} {there}"))?;
    eprintln!("Copying this checkout to {host}:~/{there}…");
    let copied = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{{ git ls-files; git ls-files --others --exclude-standard; }} | sort -u | while read -r f; do [ -e \"$f\" ] && echo \"$f\"; done | tar -cz -T - | ssh -o BatchMode=yes {host} 'mkdir -p {there} && tar -xz -C {there}'"))
        .current_dir(checkout)
        .status()?;
    anyhow::ensure!(copied.success(), "couldn't copy this checkout to {host}");
    // The copy says which port is its network's: it's in use while that port answers there, through the forward below, and
    // the next throwaway or update on that Mac deletes it once it doesn't.
    let port = |a: &str| a.trim().rsplit_once(':').and_then(|(_, p)| p.trim_end_matches('/').parse::<u16>().ok());
    ssh(&format!("echo {} > {there}/.throwaway-port && cd {there} && native-apps/disk.sh sweep", port(address).unwrap_or_default()))?;
    // The throwaway network's address and its bucket, at the same ports on the Mac, for as long as it runs.
    let mut forward = vec!["-N".to_string(), "-o".into(), "BatchMode=yes".into(), "-o".into(), "ExitOnForwardFailure=yes".into()];
    for p in [port(address), std::fs::read_to_string(dir.join("bucket-addr")).ok().as_deref().and_then(port)].into_iter().flatten() {
        forward.extend(["-R".to_string(), format!("{p}:127.0.0.1:{p}")]);
    }
    forward.push(host.clone());
    if let Some(pid) = std::fs::read_to_string(dir.join("mac-forward.pid")).ok().and_then(|p| p.trim().parse::<i32>().ok()) {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
        std::thread::sleep(Duration::from_millis(500));
    }
    let tunnel = spawn(dir, std::process::Command::new("ssh").args(&forward), "mac-forward.log", true)?;
    std::fs::write(dir.join("mac-forward.pid"), tunnel.to_string())?;
    // Its own control port: the Mac app and the iPhone app's simulator share the Mac's loopback, and run together.
    let script = if iphone { "iphone" } else { "mac" };
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(format!("{}\n{host}\n{script}", dir.display()).as_bytes()).into();
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    eprintln!("Building native-apps on {host} (dev profile; the first time takes minutes)…");
    // The Mac's window is a view of a computer of its own there, whose onet answers beside its control port.
    let last = if iphone { simulator_name(dir) } else { format!("127.0.0.1:{}", control + 1) };
    let key = platform::mode::Throwaway::own_release_key(dir)?.public_key().to_openssh()?;
    let build = format!("cd {there} && CARGO_TERM_COLOR=never PATH=$HOME/.cargo/bin:/opt/homebrew/bin:$PATH RELEASE_KEY={} native-apps/{script}/throwaway.sh '{address}' {control} {}", shell_quote(&key), shell_quote(&last));
    let said = build_on(&host, &build, &dir.join(format!("{script}-build.log")), started + Duration::from_secs(crate::ship::CALL - 90))?;
    let last: Vec<&str> = said.lines().last().unwrap_or_default().split(' ').collect();
    // The iPhone's says its simulator; the Mac's what to publish, and its window's launch job, started once it is.
    let (sim, log) = match last.as_slice() {
        [sim, log] if iphone => (sim.to_string(), log.to_string()),
        [log, publish, job] if !iphone => {
            publish_mac(dir, &host, publish)?;
            ssh(&format!("launchctl bootstrap gui/$(id -u) {}", shell_quote(job)))?;
            (String::new(), log.to_string())
        }
        _ => anyhow::bail!("on {host}, the build didn't say where it put what it built: {said}"),
    };
    let eval = |js: &str| ssh(&format!("curl -s -m 5 -d {} http://127.0.0.1:{control}/eval", shell_quote(js))).map(drop);
    let start = Instant::now();
    let (mut signed, mut approved) = (false, false);
    loop {
        let said = ssh(&format!("cat '{log}' 2>/dev/null || true"))?;
        if start.elapsed() > Duration::from_secs(180) {
            // The app's own words (its log file, which a launch that never wrote leaves empty), then the system's of it: the
            // simulator's unified log, which is where an iPhone app's prints go (without WebKit's chatter), or the Mac's launch job.
            let (what, system, whose) = if iphone {
                let predicate = format!("process == \"{NAME}\" && !(subsystem BEGINSWITH \"com.apple.\")");
                let shown = ssh(&format!("xcrun simctl spawn {sim} log show --last 5m --style compact --predicate '{predicate}' 2>&1 | grep -v -e '^Filtering' -e '^Timestamp' -e SecItemCopyMatching -e 'Loading Preferences' | tail -n 20 || true"));
                ("iPhone app", shown.unwrap_or_default(), "the simulator's log")
            } else {
                let job = ssh(&format!("launchctl print gui/$(id -u)/app.{CLI}.mac.throwaway 2>&1 | grep -E 'state|last exit|runs' || true"));
                ("Mac app", job.unwrap_or_default(), "its launch job")
            };
            return Err(didnt_sign_in(what, &[("the app", said), (whose, system)], &format!("{host}:{log} is empty or missing, so it may not have started: look at it with ssh {host}")));
        }
        std::thread::sleep(Duration::from_secs(2));
        let said = ssh(&format!("cat '{log}' 2>/dev/null || true"))?;
        if said.lines().any(|l| l.trim() == "Let in.") {
            break;
        }
        match said.lines().rev().find_map(|l| l.trim().strip_prefix("Sign in: ")) {
            // Approved as the user, then back to the app at its loopback port, which only the Mac reaches.
            Some(url) if !signed => {
                let back = crate::throwaway::sign_in::approved_on_throwaway(address, url, None)?;
                ssh(&format!("curl -s -m 30 -o /dev/null {}", shell_quote(&back)))?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the user.
            None => drop(eval(super::PRESS_SIGN_IN)),
        }
        if let Some(code) = said.lines().rev().find_map(|l| l.trim().strip_prefix("Code: "))
            && !approved
        {
            approve_code(dir, code)?;
            approved = true;
        }
    }
    if iphone {
        println!(
            "The iPhone app of this build is signed in to the throwaway network, on a simulator of its own on {host} ({sim}), in dark with the text a size smaller, as a phone's settings are.
  See it: ssh {host} xcrun simctl io {sim} screenshot /tmp/shot.png
  Use it: in its page there: ssh {host} curl -d '<js>' http://127.0.0.1:{control}/eval runs JavaScript, and /value gives an expression's value, as JSON; open a link: ssh {host} xcrun simctl openurl {sim} '{CLI}://…'
  What it said: {host}:{log}; {CLI}-dev throwaway stop shuts the simulator down and deletes it"
        );
        return Ok(());
    }
    println!(
        "The Mac app of this build is signed in to the throwaway network, on {host} beside the app there (its menu bar item, its window).
  See it: ssh {host} screencapture -x /tmp/shot.png
  Use it: in its page there: ssh {host} curl -d '<js>' http://127.0.0.1:{control}/eval runs JavaScript, and /value gives an expression's value, as JSON (/eval/panel and /value/panel, the panel's)
  What it said: {host}:{log}; {CLI}-dev throwaway stop stops it"
    );
    Ok(())
}

/// Publishes what `native-apps/mac/throwaway.sh` built on `host` in `folder` for a Mac at the throwaway network's address,
/// under its own release key, as a Mac publishes a real build (`oned::publish::mac_things`): the program, which the window
/// sets up its computer from, the helper apps its oned runs, and the window, which `update` takes.
fn publish_mac(dir: &Path, host: &str, folder: &str) -> Result<()> {
    let here = dir.join("mac-publish");
    let version = crate::build::mac_build_from(host, folder, &here)?;
    let addr = std::fs::read_to_string(dir.join("addr"))?;
    let things = oned::publish::mac_things(&here, &version, "-")?;
    oned::publish::publish_latest(&crate::throwaway::loopback_at(dir, addr.trim()), &platform::mode::Throwaway::own_release_key(dir)?, &things)?;
    eprintln!("Published {} for {} at the throwaway network's address: its program, helpers and window.", version.commit, version.target);
    Ok(())
}

/// Runs the build on `host` and gives its last line, in a session of its own that the Mac hangs up when its ssh ends, for any
/// reason, and the build with it, so none is left running there when this is killed or the connection drops. At `deadline`,
/// before the agent's harness would cut this call off, it ends the build the same way, saying so: what it built stays, and
/// the same command again builds on from there.
fn build_on(host: &str, command: &str, log: &Path, deadline: Instant) -> Result<String> {
    #[cfg(target_os = "linux")]
    use std::os::unix::process::CommandExt;
    let mut ssh = std::process::Command::new("ssh");
    // -tt: ssh without a terminal gives the command's processes no hangup when it ends; with one the Mac sends it to them all.
    // The terminal merges the command's errors into what it printed. Its own death takes ssh with it, as a kill that leaves it
    // no time to end it would otherwise leave the build running.
    ssh.args(["-tt", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=15", host, command]).stdin(std::process::Stdio::null()).stdout(std::fs::File::create(log)?).stderr(std::process::Stdio::null());
    // macOS has no PR_SET_PDEATHSIG, and one-dev is built there too.
    #[cfg(target_os = "linux")]
    unsafe { ssh.pre_exec(|| if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }) };
    let mut ssh = ssh.spawn().with_context(|| format!("starting ssh to {host}"))?;
    let status = loop {
        if let Some(status) = ssh.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            ssh.kill()?;
            ssh.wait()?;
            anyhow::bail!("{host} was still building when this ran out of time, so it stopped the build there (what it built stays: it would otherwise run on after the agent's call was cut off). The same command again builds on from here. What it said so far: {}", log.display());
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let said = std::fs::read_to_string(log)?.replace('\r', "");
    anyhow::ensure!(status.success(), "on {host}: {command}: {}", said.trim());
    Ok(said.trim().to_string())
}

/// The name of the iPhone simulator this throwaway network made, which no other has (its folder's name is its checkout's, and
/// a hash of its path): what `stop` finds it by, so it never takes down another's.
fn simulator_name(dir: &Path) -> String {
    format!("{CLI} throwaway {}", dir.file_name().unwrap_or_default().to_string_lossy())
}

/// Takes down what `client mac` and `client iphone` started on the Mac: the Mac app, and the iPhone app's simulator, and
/// with `delete`, the copy of this checkout they were built from.
pub(crate) fn stop(dir: &Path, delete: bool) {
    let on = |file: &str, mut command: String| {
        let Ok(said) = std::fs::read_to_string(dir.join(file)) else { return };
        let mut words = said.split_whitespace();
        let host = words.next().unwrap_or_default();
        if delete && let Some(there) = words.next() {
            command += &format!("; rm -rf {}", shell_quote(there));
        }
        match std::process::Command::new("ssh").args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, &command]).output() {
            Ok(out) if out.status.success() => {}
            Ok(out) => eprintln!("Couldn't take down the {file} client on {host}: {}", String::from_utf8_lossy(&out.stderr).trim()),
            Err(err) => eprintln!("Couldn't take down the {file} client on {host}: {err}"),
        }
        let _ = std::fs::remove_file(dir.join(file));
    };
    // Its window, and its computer's oned, a launch agent named for the computer's home (`platform::set_up`).
    on("mac", format!("launchctl bootout gui/$(id -u)/app.{CLI}.mac.throwaway; for agent in ~/Library/Caches/{CLI}/throwaway.noindex/native-apps/target/throwaway-home/app.{CLI}.{CLI}d.throwaway-*.plist; do [ -e \"$agent\" ] && launchctl bootout gui/$(id -u) \"$agent\"; done; true"));
    // Every simulator of that name (a run that stopped before it said which): by its id, as a name is only what's shown.
    let name = shell_quote(&simulator_name(dir));
    on("iphone", format!("PATH=/opt/homebrew/bin:$PATH; for sim in $(xcrun simctl list devices -j | jq -r --arg n {name} '.devices[][] | select(.name == $n) | .udid'); do xcrun simctl shutdown $sim; xcrun simctl delete $sim; done"));
}
