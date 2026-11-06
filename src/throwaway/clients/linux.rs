//! The Linux client: this checkout's client/native, on a display of its own.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use one::config::CLI;

use super::super::devices::{approve_code, didnt_sign_in};
use super::super::network::spawn;
use crate::agent;

/// The native app (client/native) as this checkout builds it, on this machine in a display of its own (Xvfb), signed in
/// to the throwaway network as its first person or `person` as a person would: its Sign In pressed, the sign-in it would
/// open in a browser approved as them, and the code it then shows approved by the throwaway network's machine. What it
/// keeps (its folder, and its key in the Secret Service by the throwaway network's address) stays with the throwaway network, so a
/// new build is still signed in; it's started again each time.
pub(crate) fn linux(dir: &Path, checkout: &Path, settings: &one::cloudflare::Settings, person: Option<&str>) -> Result<()> {
    let address = settings.address.as_str();
    let name = person.map(|p| format!("linux-{}", p.to_lowercase().replace(' ', "-"))).unwrap_or_else(|| "linux".into());
    let home = dir.join(&name);
    std::fs::create_dir_all(&home)?;
    eprintln!("Building client/native (dev profile; the first time takes minutes)…");
    crate::build::quietly(std::process::Command::new("nice").arg(crate::build::cargo()).arg("build").current_dir(checkout.join("client/native")))?;
    // Its own display and control port, the same each time for this throwaway network and person.
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(home.to_string_lossy().as_bytes()).into();
    let display = format!(":{}", 200 + u16::from_le_bytes([hash[0], hash[1]]) % 600);
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    // What ran before for it goes: the app, and its display.
    for file in ["app.pid", "display.pid"] {
        if let Some(pid) = std::fs::read_to_string(home.join(file)).ok().and_then(|p| p.trim().parse::<i32>().ok()) {
            unsafe { libc::kill(-pid, libc::SIGTERM) };
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    let xvfb = spawn(dir, std::process::Command::new("Xvfb").args([display.as_str(), "-screen", "0", "1280x800x24", "-nolisten", "tcp"]), &format!("{name}/display.log"), true)?;
    std::fs::write(home.join("display.pid"), xvfb.to_string())?;
    std::thread::sleep(Duration::from_secs(1));
    let env = |key: &str| format!("{}_{key}", CLI.to_uppercase());
    let log = format!("{name}/app.log");
    let app = spawn(
        dir,
        // On a session bus of its own, with a Secret Service there (GNOME's keyring, unlocked with a password of no worth, in its
        // folder), as a desktop has.
        std::process::Command::new("dbus-run-session")
            .args(["--", "sh", "-c", "printf throwaway | gnome-keyring-daemon --unlock --components=secrets >/dev/null && exec \"$0\""])
            .arg(checkout.join("client/native/target/debug/one-native"))
            .env("DISPLAY", &display)
            .env_remove("WAYLAND_DISPLAY")
            // What it writes of itself (its launcher entry, open at login, what opens `<cli>://`) stays in its folder.
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
            .env(env("ADDRESS"), address)
            .env(env("SIGN_IN_PRINT"), "1")
            .env(env("CONTROL"), control.to_string()),
        &log,
        true,
    )?;
    std::fs::write(home.join("app.pid"), app.to_string())?;
    let eval = |js: &str| agent().post(format!("http://127.0.0.1:{control}/eval")).send(js).map(drop);
    let start = Instant::now();
    let (mut signed, mut approved) = (false, false);
    loop {
        let said = std::fs::read_to_string(dir.join(&log)).unwrap_or_default();
        if start.elapsed() > Duration::from_secs(180) {
            // Its display's words (Xvfb's) only if the app has none: they're mostly keyboard warnings.
            let display_said = if said.trim().is_empty() { std::fs::read_to_string(dir.join(format!("{name}/display.log"))).unwrap_or_default() } else { String::new() };
            return Err(didnt_sign_in("Linux app", &[("the app", said), ("its display", display_said)], &format!("{} is empty, so it may not have started", dir.join(&log).display())));
        }
        std::thread::sleep(Duration::from_secs(1));
        let said = std::fs::read_to_string(dir.join(&log)).unwrap_or_default();
        if said.lines().any(|l| l == "Let in.") {
            break;
        }
        match said.lines().rev().find_map(|l| l.strip_prefix("Sign in: ")) {
            Some(url) if !signed => {
                one::sign_in::approve_on_throwaway(address, url, person)?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the person.
            None => drop(eval(super::PRESS_SIGN_IN)),
        }
        if let Some(code) = said.lines().rev().find_map(|l| l.strip_prefix("Code: "))
            && !approved
        {
            approve_code(dir, code)?;
            approved = true;
        }
    }
    println!(
        "The Linux app of this build is signed in to the throwaway network{}, on this machine in the display {display}.
  See it: DISPLAY={display} import -window root shot.png
  Use it: DISPLAY={display} xdotool …, or in its page: curl -d '<js>' http://127.0.0.1:{control}/eval runs JavaScript, and /value gives an expression's value, as JSON
  What it said: {}; {CLI} throwaway stop stops it",
        person.map(|name| format!(" as {name}")).unwrap_or_default(),
        dir.join(&log).display()
    );
    Ok(())
}
