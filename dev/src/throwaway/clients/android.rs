//! The Android client, on a throwaway copy of a borrowed device.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use ::custom_build::config::CLI;
use serde_json::json;

use super::super::devices::{agent_devices, approve_code, build, didnt_sign_in, on_device};

/// The Android app for this machine's processor, built in the background: → its APK's path, once `android` joins it.
pub(crate) fn build_here(dir: &Path, checkout: &Path) -> std::thread::JoinHandle<Result<String>> {
    let (dir, checkout) = (dir.to_path_buf(), checkout.to_path_buf());
    std::thread::spawn(move || apk(&dir, &checkout, abi_here()))
}

fn abi_here() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64-v8a" } else { "x86_64" }
}

fn apk(dir: &Path, checkout: &Path, abi: &str) -> Result<String> {
    let said = build(dir, checkout, "native-apps/android/build.sh", &[("ABIS", abi)])?;
    Ok(said.lines().last().unwrap_or_default().to_string())
}

/// The Android app (native-apps), as this checkout builds it (native-apps/android/build.sh, for the processor the device
/// has), installed over the one there, then started with the throwaway network's address, printing its sign-in rather than
/// opening a Custom Tab, and with a control port, as linux() starts it (its settings go in as the intent's `env.` extras,
/// which a debug build reads): its Sign In pressed, the sign-in approved as the throwaway network's first user or `user`,
/// coming back to it by the app's own link, opened on the device, and its code approved by the throwaway network's machine.
/// Once signed in on a device as them (a file in the throwaway network's folder says so), a new build keeps that sign-in.
/// `early` is the app built for this machine's processor while the device was being lent (`build_here`): an emulator here
/// has it, as copies usually are. A device of another is built for again.
pub(crate) fn android(dir: &Path, checkout: &Path, device: &str, settings: &oned_cloudflare::Settings, user: Option<&str>, early: std::thread::JoinHandle<Result<String>>) -> Result<()> {
    let address = settings.address.as_str();
    let abi = match on_device(device, "getprop ro.product.cpu.abi")?.trim() {
        abi @ ("x86_64" | "arm64-v8a") => abi.to_string(),
        other => bail!("this checkout builds the Android app for x86_64 and arm64-v8a, not {other}"),
    };
    let early = early.join().map_err(|_| anyhow::anyhow!("building the Android app stopped"))?;
    let apk = match abi == abi_here() {
        true => early?,
        false => apk(dir, checkout, &abi)?,
    };
    let package = ::custom_build::config::ANDROID;
    agent_devices("install", json!({ "device": device, "app": apk }))?;
    // The launcher keeps drawing home-screen widgets from the APK it replaced ("Can't load widget") until it restarts.
    on_device(device, "am force-stop com.google.android.apps.nexuslauncher")?;
    // Play Store updates WebView on a fresh copy a few minutes in, which kills every app using it: not while a test runs.
    on_device(device, "pm disable-user --user 0 com.android.vending")?;
    let signed_in = dir.join(format!("signed-in-{device}"));
    let first = settings.throwaway_user.as_ref().map(|(_, name)| name.as_str()).unwrap_or_default();
    let who = user.unwrap_or(first);
    let here = format!("{address}\n{who}");
    let kept = std::fs::read_to_string(&signed_in).is_ok_and(|was| was == here);
    // A copy of the user's phone may have a build signed in elsewhere, or as someone else: what it kept goes.
    if !kept {
        on_device(device, &format!("pm clear {package}"))?;
    }
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(format!("{}\n{device}", dir.display()).as_bytes()).into();
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    let env = |key: &str| ::custom_build::config::env(key);
    on_device(device, "logcat -c")?;
    on_device(
        device,
        &format!(
            "am force-stop {package}; am start -W -n {package}/app.client.MainActivity --es env.{} '{address}' --es env.{} 1 --es env.{} {control}",
            env("ADDRESS"),
            env("SIGN_IN_PRINT"),
            env("CONTROL"),
        ),
    )?;
    // Its page's JavaScript, run through the control port, which only the device reaches.
    let eval = |js: &str| on_device(device, &format!("(printf 'POST /eval HTTP/1.0\\r\\ncontent-length: {}\\r\\n\\r\\n'; printf '%s' \"{js}\") | nc -w 2 127.0.0.1 {control}", js.len()));
    let start = Instant::now();
    let (mut signed, mut approved) = (false, false);
    loop {
        if start.elapsed() > Duration::from_secs(180) {
            let said = on_device(device, "logcat -d -s RustStdoutStderr:I")?;
            let said: Vec<&str> = said.lines().filter_map(|l| l.split_once("RustStdoutStderr: ").map(|(_, s)| s.trim())).collect();
            let crashed = on_device(device, "logcat -d -b crash").unwrap_or_default();
            return Err(didnt_sign_in("Android app", &[("the app", said.join("\n")), ("Android's crash log", crashed)], &format!("look at it with {CLI} computer screen --device {device}")));
        }
        std::thread::sleep(Duration::from_secs(2));
        let said = on_device(device, "logcat -d -s RustStdoutStderr:I")?;
        let said: Vec<&str> = said.lines().filter_map(|l| l.split_once("RustStdoutStderr: ").map(|(_, s)| s.trim())).collect();
        if said.contains(&"Let in.") {
            break;
        }
        match said.iter().rev().find_map(|l| l.strip_prefix("Sign in: ")) {
            // Approved as the user, then back to the app by its own link, which opens it on the device.
            Some(url) if !signed => {
                let back = crate::throwaway::sign_in::approved_on_throwaway(address, url, user)?;
                on_device(device, &format!("am start -a android.intent.action.VIEW -n {package}/app.client.MainActivity -d '{back}'"))?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the user.
            None => drop(eval(super::PRESS_SIGN_IN)),
        }
        if let Some(code) = said.iter().rev().find_map(|l| l.strip_prefix("Code: "))
            && !approved
        {
            // Someone else's device is approved by one of theirs (their browser, which joined first), not this machine's.
            match approve_code(dir, code) {
                Err(err) if user.is_some() && format!("{err:#}").contains("one of their own devices approves it") => {
                    println!(
                        "The Android app of this build is signed in to the throwaway network as {who}, on the device {device}, and waits for one of {who}'s own devices to let it in.
  Approve it as {who}: in their browser (`{CLI}-dev throwaway client web --as {who}` makes it), open {address}/approve#{}, or {CLI} computer screen --device {device} for the code",
                        code.replace(' ', "")
                    );
                    return Ok(());
                }
                approved => approved?,
            }
            approved = true;
        }
    }
    std::fs::write(&signed_in, &here)?;
    println!(
        "  In its page, POST /eval runs JavaScript and POST /value gives an expression's value, as JSON: {CLI} agent-devices run --device {device} \"(printf 'POST /eval HTTP/1.0\\r\\ncontent-length: <length>\\r\\n\\r\\n'; printf '%s' '<js>') | nc -w 2 127.0.0.1 {control}\"; what it said: {CLI} agent-devices run --device {device} \"logcat -d -s RustStdoutStderr\"; open a link: {CLI} agent-devices run --device {device} \"am start -a android.intent.action.VIEW -d '{CLI}://…' {package}\""
    );
    Ok(())
}
