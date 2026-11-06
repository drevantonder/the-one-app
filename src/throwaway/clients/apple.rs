//! The Mac app, and the iPhone app on a simulator of its own, built and run over ssh on the Mac.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use one::config::{CLI, NAME};

use super::super::devices::{approve_code, didnt_sign_in, shell_quote};
use super::super::network::spawn;

/// The Mac app (client/native), or with `iphone` the iPhone app on a simulator, as this checkout builds it, on the Mac `MAC` names to ssh (`mac` if none), beside the
/// person's own app there: a throwaway's build (its own id, folder and Keychain items), in the Mac's logged-in session,
/// reaching the throwaway network at its address through ports ssh forwards there. Signed in as the throwaway network's
/// first person as linux() is: its Sign In pressed, the sign-in approved as them and opened there, its code approved.
/// What it keeps stays on the Mac, so a new build is still signed in; it's started again each time.
pub(crate) fn apple(dir: &Path, checkout: &Path, settings: &one::cloudflare::Settings, iphone: bool) -> Result<()> {
    let address = settings.address.as_str();
    let host = std::env::var("MAC").unwrap_or_else(|_| "mac".into());
    std::fs::write(dir.join(if iphone { "iphone" } else { "mac" }), &host)?;
    let ssh = |command: &str| -> Result<String> {
        let out = std::process::Command::new("ssh").args(["-o", "BatchMode=yes", &host, command]).output()?;
        anyhow::ensure!(out.status.success(), "on {host}: {command}: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    // This checkout as it is now, tracked and new files, in the Mac's cache, where its builds stay between runs.
    let there = format!("Library/Caches/{CLI}/throwaway-{}", checkout.file_name().unwrap_or_default().to_string_lossy());
    eprintln!("Copying this checkout to {host}:~/{there}…");
    let copied = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{{ git ls-files; git ls-files --others --exclude-standard; }} | sort -u | while read -r f; do [ -e \"$f\" ] && echo \"$f\"; done | tar -cz -T - | ssh -o BatchMode=yes {host} 'mkdir -p {there} && tar -xz -C {there}'"))
        .current_dir(checkout)
        .status()?;
    anyhow::ensure!(copied.success(), "couldn't copy this checkout to {host}");
    // The throwaway network's address and its bucket, at the same ports on the Mac, for as long as it runs.
    let port = |a: &str| a.trim().rsplit_once(':').and_then(|(_, p)| p.trim_end_matches('/').parse::<u16>().ok());
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
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(format!("{}\n{host}", dir.display()).as_bytes()).into();
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    eprintln!("Building client/native on {host} (dev profile; the first time takes minutes)…");
    let script = if iphone { "iphone" } else { "mac" };
    let said = ssh(&format!("cd {there} && PATH=$HOME/.cargo/bin:/opt/homebrew/bin:$PATH client/native/{script}/throwaway.sh '{address}' {control}"))?;
    // The iPhone's says its simulator too.
    let (sim, log) = match said.lines().last().unwrap_or_default().rsplit_once(' ') {
        Some((sim, log)) if iphone => (sim.to_string(), log.to_string()),
        _ => (String::new(), said.lines().last().unwrap_or_default().to_string()),
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
            // Approved as the person, then back to the app at its loopback port, which only the Mac reaches.
            Some(url) if !signed => {
                let back = crate::sign_in::approved_on_throwaway(address, url, None)?;
                ssh(&format!("curl -s -m 30 -o /dev/null {}", shell_quote(&back)))?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the person.
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
  What it said: {host}:{log}; {CLI} throwaway stop shuts the simulator down and deletes it"
        );
        return Ok(());
    }
    println!(
        "The Mac app of this build is signed in to the throwaway network, on {host} beside the app there (its menu bar item, its window).
  See it: ssh {host} screencapture -x /tmp/shot.png
  Use it: in its page there: ssh {host} curl -d '<js>' http://127.0.0.1:{control}/eval runs JavaScript, and /value gives an expression's value, as JSON (/eval/panel and /value/panel, the panel's)
  What it said: {host}:{log}; {CLI} throwaway stop stops it"
    );
    Ok(())
}
