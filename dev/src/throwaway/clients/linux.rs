//! The Linux client: this checkout's native-apps, on a display of its own, the window of a computer of its own.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use ::custom_build::config::CLI;

use super::super::devices::{approve_code, didnt_sign_in};
use super::super::network::{shares_native_build, spawn};
use crate::agent;

/// The native app (native-apps) as this checkout builds it, on this machine in a display of its own (Xvfb), the window
/// of a computer of its own (a home, `computer`, and an address its onet answers at), signed in to the throwaway network
/// as its first user or `user` as a user would: its Sign In pressed, which sets up `one` on that computer the first time,
/// from this build, published at the throwaway network's address; the sign-in it would open in a browser approved as
/// them; and the code the computer then shows approved by the throwaway network's machine, or `user`'s browser. The
/// computer's oned runs under this user's systemd, as a unit of the throwaway network's (`stop`), and it and what the app
/// keeps stay with the throwaway network, so a new build is still in; the app is started again each time.
pub(crate) fn linux(dir: &Path, checkout: &Path, settings: &oned_cloudflare::Settings, user: Option<&str>) -> Result<()> {
    let address = settings.address.as_str();
    let name = user.map(|p| format!("linux-{}", p.to_lowercase().replace(' ', "-"))).unwrap_or_else(|| "linux".into());
    let home = dir.join(&name);
    let computer = home.join("computer");
    std::fs::create_dir_all(&computer)?;
    // Where the computer's onet answers, the same each time.
    let addr = match std::fs::read_to_string(home.join("computer-addr")) {
        Ok(addr) => addr.trim().to_string(),
        Err(_) => {
            let addr = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string();
            std::fs::write(home.join("computer-addr"), &addr)?;
            addr
        }
    };
    if !computer.join("bin").join(CLI).exists() {
        eprintln!("Publishing this build at the throwaway network's address, for the app to set up its computer from…");
        crate::throwaway::publish_build(dir, std::fs::read_to_string(dir.join("addr"))?.trim())?;
    }
    eprintln!("Building native-apps (dev profile; the first time takes minutes)…");
    let mut cargo = platform::os::nice_cargo();
    shares_native_build(dir, &mut cargo);
    platform::os::quietly(cargo.arg("build").current_dir(checkout.join("native-apps")))?;
    // Its own display and control port, the same each time for this throwaway network and user.
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
    let env = |key: &str| ::custom_build::config::env(key);
    let log = format!("{name}/app.log");
    // On a session bus of its own, with a Secret Service there (GNOME's keyring, unlocked with a password of no worth, in its
    // folder), as a desktop has; told its computer's home and address as the throwaway network's programs are.
    let mut command = std::process::Command::new("dbus-run-session");
    platform::mode::Throwaway::tell(&mut command, &computer, &addr);
    let app = spawn(
        dir,
        command
            .args(["--", "sh", "-c", "printf throwaway | gnome-keyring-daemon --unlock --components=secrets >/dev/null && exec \"$0\""])
            .arg(checkout.join("native-apps/target/debug/one-native"))
            .env(env("RELEASE_KEY"), platform::mode::Throwaway::own_release_key(dir)?.public_key().to_openssh()?)
            .env("DISPLAY", &display)
            .env_remove("WAYLAND_DISPLAY")
            // What it writes of itself (its launcher entry, open at login, what opens `<cli>://`) stays in its folder.
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
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
                crate::throwaway::sign_in::approve_on_throwaway(address, url, user)?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the user.
            None => drop(eval(super::PRESS_SIGN_IN)),
        }
        if let Some(code) = said.lines().rev().find_map(|l| l.strip_prefix("Code: "))
            && !approved
        {
            match user {
                // Someone else's device is let in by one of theirs: their browser, which joined first.
                Some(name) => {
                    let port = super::web::browser(dir, settings, Some(name))?.to_string();
                    super::web::web_script(dir, &["approve", &port, address, "", code])?;
                }
                None => approve_code(dir, code)?,
            }
            approved = true;
        }
    }
    println!(
        "The Linux app of this build is signed in to the throwaway network{}, on this machine in the display {display}, the window of a computer whose home is {}.
  See it: DISPLAY={display} import -window root shot.png
  Use it: DISPLAY={display} xdotool …, or in its page: curl -d '<js>' http://127.0.0.1:{control}/eval runs JavaScript, and /value gives an expression's value, as JSON
  What it said: {}; {CLI}-dev throwaway stop stops it",
        user.map(|name| format!(" as {name}")).unwrap_or_default(),
        computer.display(),
        dir.join(&log).display()
    );
    Ok(())
}

/// Stops the computers the Linux app set up for the throwaway network in `dir`: their oned, each a unit of this user's
/// systemd whose file is in the computer's home, as the app writes it there.
pub(crate) fn stop(dir: &Path) {
    let Ok(folders) = std::fs::read_dir(dir) else { return };
    for computer in folders.flatten().map(|f| f.path().join("computer")) {
        let Ok(files) = std::fs::read_dir(&computer) else { continue };
        for unit in files.flatten().filter_map(|f| f.file_name().to_str().filter(|n| n.starts_with(&format!("{CLI}d-throwaway-")) && n.ends_with(".service")).map(String::from)) {
            let _ = std::process::Command::new("systemctl").args(["--user", "disable", "--now", &unit]).output();
        }
    }
}
