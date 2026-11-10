//! `throwaway client <kind>`: a client of this checkout's build, signed in to the throwaway network.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use one::config::CLI;
use serde_json::{Value, json};

use super::devices::{agent_devices, borrowed_for, devices, listed_devices};
use super::network::client_session;

mod android;
pub(super) mod apple;
mod linux;
mod web;
mod windows;

/// What a client signed out is told in its page to press Sign In, the same in each one's screen.
const PRESS_SIGN_IN: &str = "document.querySelector('[data-slot=welcome-action]')?.click()";

/// `throwaway client <kind> [--as <name>]`: a client of this checkout's build, signed in to the throwaway network as its first
/// user, or as someone else they invited, and how to see and use it. Again, it's the same client, with this
/// checkout's build as it is now.
pub(crate) fn client(dir: &Path, checkout: &Path, settings: &one::cloudflare::Settings, rest: &[String]) -> Result<()> {
    let started = Instant::now();
    // A session while it runs: a new build waits for the next command with none to start the throwaway network again.
    let _session = client_session(dir);
    let user = match rest.iter().position(|a| a == "--as") {
        Some(at) => Some(user_named(rest.get(at + 1).context("--as who? --as sam")?)?),
        None => None,
    };
    // As its first user is as nobody else.
    let first = settings.throwaway_user.as_ref().map(|(_, name)| name.to_lowercase());
    let user = user.filter(|name| Some(name.to_lowercase()) != first);
    match rest.first().map(String::as_str) {
        Some("web") => web::web(dir, settings, user.as_deref())?,
        Some("linux") => {
            if let Some(name) = &user {
                web::joined(dir, settings, name)?;
            }
            linux::linux(dir, checkout, settings, user.as_deref())?
        }
        Some(kind @ ("mac" | "iphone")) => {
            anyhow::ensure!(user.is_none(), "the {kind} client signs in only as the throwaway network's first user: use web, linux or android for someone else");
            apple::apple(dir, checkout, settings, kind == "iphone")?
        }
        Some(kind @ ("android" | "windows")) => {
            if let Some(name) = &user {
                // They join where anyone does, in a browser: the throwaway network's own here.
                web::joined(dir, settings, name)?;
            }
            device_client(dir, checkout, settings, kind, user.as_deref())?
        }
        _ => bail!("which client? {CLI}-dev throwaway client web, linux, mac, iphone, android or windows, with --as <name> for someone else"),
    }
    println!("Ready in {}s.", started.elapsed().as_secs());
    Ok(())
}

/// Someone's name as `--as` gives it, as users write it: "sam" is Sam.
pub(crate) fn user_named(given: &str) -> Result<String> {
    let given = given.trim();
    anyhow::ensure!(!given.is_empty() && given.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-'), "--as takes a name, like --as sam");
    let mut chars = given.chars();
    Ok(chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default())
}

/// An Android or Windows client: on a throwaway copy of the user's lasting device, borrowed for the throwaway network,
/// which reaches the throwaway network's address at the same address; this checkout's build installed there, and signed in.
fn device_client(dir: &Path, checkout: &Path, settings: &one::cloudflare::Settings, kind: &str, user: Option<&str>) -> Result<()> {
    let mut borrowed = devices(dir);
    let what = borrowed_for(checkout, kind);
    let listed = listed_devices;
    // A copy borrowed for this checkout's throwaway network that it lost track of (the node restarted as it was lent) is
    // taken, not left beside a second one.
    let ours = |listed: &Value| listed.as_array().into_iter().flatten().find(|d| d["borrowed"]["for"] == what.as_str()).and_then(|d| d["name"].as_str()).map(String::from);
    let now = listed();
    let still = |name: &str| now.as_array().into_iter().flatten().any(|d| d["name"] == name);
    let device = match borrowed.get(kind).and_then(Value::as_str).filter(|d| still(d)).map(String::from).or_else(|| ours(&now)) {
        Some(device) => device,
        None => {
            eprintln!("Borrowing a copy of your {kind} device…");
            let got = match agent_devices("borrow", json!({ "kind": kind, "copy": true, "for": what })) {
                Ok(got) => got,
                Err(err) => match ours(&listed()) {
                    Some(device) => json!({ "device": device }),
                    None => return Err(err),
                },
            };
            let device = got["device"].as_str().context("Agent Devices lent no device")?.to_string();
            borrowed.insert(kind.into(), json!(device));
            std::fs::write(dir.join("devices.json"), serde_json::to_vec(&borrowed)?)?;
            device
        }
    };
    let port = settings.ports[1];
    agent_devices("reach", json!({ "device": device, "port": port }))?;
    // A native client reads who's who from the bucket itself, at the URLs the signer gives it: the throwaway bucket's port.
    if let Some(bucket) = std::fs::read_to_string(dir.join("bucket-addr")).ok().and_then(|a| a.trim().rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok())) {
        agent_devices("reach", json!({ "device": device, "port": bucket }))?;
    }
    // A copy is given back after 15 minutes without a call: a build may take longer.
    let building = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let keep = {
        let (building, device) = (building.clone(), device.clone());
        std::thread::spawn(move || {
            while building.load(std::sync::atomic::Ordering::Relaxed) {
                for _ in 0..300 {
                    std::thread::sleep(Duration::from_secs(1));
                    if !building.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                }
                let _ = agent_devices("reach", json!({ "device": device, "port": port }));
            }
        })
    };
    let built = match kind {
        "android" => android::android(dir, checkout, &device, settings, user),
        _ => windows::windows(dir, checkout, &device, settings, user),
    };
    building.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = keep.join();
    built?;
    println!(
        "The {kind} client of this build is signed in to the throwaway network{}, on the device {device}, a throwaway copy borrowed for it.
  See it: {CLI} computer screen --device {device} (computer_screen), or {CLI} computer view --device {device} for a link
  Use it: computer_use with device {device} (computer_tools lists what it takes); {CLI}-dev throwaway stop gives it back",
        user.map(|name| format!(" as {name}")).unwrap_or_default()
    );
    Ok(())
}
